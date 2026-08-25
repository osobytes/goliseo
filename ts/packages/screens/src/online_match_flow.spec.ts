// This suite drives the online match flow end to end: a real manual
// offer/answer handshake over an in-process fake star transport, a match
// driven to full time, and an acknowledged result. The session and
// simulation are Rust-owned (`crates/gc-sim`/`crates/gc-netcode`;
// ARCHITECTURE.md §1.1); `@gc/render` (`frame_buffer`, `pitch`,
// `match_hud_render`) and `@gc/input` (`bindings`) are both declared
// dependencies of this package, used directly below. `@gc/wasm` is a
// devDependency, reachable from this spec file only -- `online_match.ts`/
// `online_match_model.ts` keep receiving a coordinator only through their
// own injected `CoordinatorPort`/`MatchDriverPort` (see those files' own
// headers). It exports a real `Coordinator`
// (`crates/gc-wasm/src/coordinator_bridge.rs`) over `gc_netcode::coordinator`'s
// actual reducer, not a fake of it.
//
// `online_match.ts` is entirely port-injected (`MatchDriverPort`,
// `MatchPresentationPort`, `MatchSessionPort`, `MatchLobbyLinkPort`,
// `LobbyFramingPort`, `MatchContractPort`, an observer port, and a
// `newMatch` factory returning `real_match.ts`'s `RealMatchScreenPort`) --
// the same pattern `match_screen.spec.ts` uses for `match.ts`/
// `SimHostPort`. Some cases below run against hand-written fakes for all
// of that, with no `@gc/wasm` dependency at all. Two run against a real
// `Coordinator`, driven through `realCoordinatorPort()`/
// `realOnlineModelPorts()` below: a solo host reaching an agreed
// "completed" result, and a real host+guest pair, relayed through
// `pump()`, proving the *other* peer's `"peer_abort"` reason is the
// coordinator's own protocol decision, not this file's invention.
//
// Several combat-family/render-output cases run for real against
// `realMatchDriverPort` -- a real `@gc/wasm` `MatchDriverBridge` (see that
// helper's own header for the one remaining gap it surfaces: no roster
// export) -- wired through a real `match.ts` `MatchScreen` in its `online`
// construction mode (`MatchScreenAsOnlineMatchScreen`), not
// `fakeMatchScreen`/`fakeMatchDriver`. The two control-ownership cases
// still use those fakes: `gc_netcode::coordinator::plan_assignments` is
// not part of `coordinator_bridge.rs`'s bound surface (same gap
// `lobby.spec.ts`'s header documents), so there is no real assignment
// algorithm to drive them against without a spec-local fake standing in
// for it -- exactly the "asserting against your own fake" this suite must
// not do. See `fakeMatchScreen`'s own header for why it still exists for
// the other cases in this file that do not need real switch-rule/render
// output.
//
// A few cases remain `it.skip`. See each one's own comment for its
// specific, current blocker (each is a real, precisely located gap -- a
// missing `@gc/wasm` binding, or a `gc_render::frame_buffer` wire-format
// gap -- not a placeholder).

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
import { MatchScreen, MatchScreenAsOnlineMatchScreen } from "./match.ts";
import type { InputSample, RenderFrame, RenderFrameRoster, RenderPort } from "./match.ts";
// `@gc/wasm` is a devDependency of this package (package.json), reachable
// from a spec file only -- see this file's header. `online_match.ts`/
// `online_match_model.ts` must keep receiving a coordinator through their
// existing injected `CoordinatorPort`s instead.
import { loadSimHost } from "@gc/wasm";
import type { Coordinator, MatchDriverBridge, SimHost, SimSession } from "@gc/wasm";
import { frameBuffer } from "@gc/render";
import type { KeyboardState } from "@gc/input";

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
    unarmed: {
      id: "unarmed",
      name: "Unarmed",
      windup_ticks: 1,
      recovery_ticks: 1,
      cooldown_ticks: 1,
      front_arc_degrees: 1,
    },
    guard: {
      id: "guard",
      name: "Guard",
      windup_ticks: 1,
      recovery_ticks: 1,
      cooldown_ticks: 1,
      front_arc_degrees: 1,
    },
    light_melee: {
      id: "light_melee",
      name: "Light Melee",
      windup_ticks: 1,
      recovery_ticks: 1,
      cooldown_ticks: 1,
      front_arc_degrees: 1,
    },
    ranged: {
      id: "ranged",
      name: "Ranged",
      windup_ticks: 1,
      recovery_ticks: 1,
      cooldown_ticks: 1,
      front_arc_degrees: 1,
    },
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
  if (event["kind"] === "link_lost") {
    // `code` is a `TransportErrorCode` (a string union) on this event; read it
    // as one rather than stringifying an `unknown`, which would silently
    // produce "[object Object]" if the shape ever changed.
    const code = event["code"];
    return typeof code === "string" ? code : "transport_lost";
  }
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

// #612: a control wire carrying an already-decided Abort and a `link_lost`
// derived from `star.pollEvent()`'s own connection-close observation can both
// be waiting in the same `drainControl()` call -- the real coordinator's
// `terminate_session` sends the Abort, then closes the link, one causal
// event reaching this screen over two independently-queued local
// observations with no guaranteed order between them. `fakeCoordinatorPort`
// above treats "control" as relay-only for every other case in this file, so
// it cannot tell that scenario apart from an ordinary in-match message; this
// one does, the way the real coordinator's `apply_abort` does -- an
// announced Abort names its own reason -- so the "which one lands first"
// distinction below is actually observable.
function abortAwareCoordinatorPort(): CoordinatorPort<FakeCoordinatorState> {
  return {
    step(state, event): readonly [FakeCoordinatorState, CoordinatorOutcome] {
      if (event["kind"] === "control") {
        const wire = event["wire"];
        const parsed =
          typeof wire === "string"
            ? (JSON.parse(wire) as { readonly kind?: string; readonly reason?: string })
            : undefined;
        if (parsed?.kind === "abort") {
          const terminal: CoordinatorTerminal = { reason: parsed.reason ?? "peer_abort" };
          return [
            { ...state, phase: "terminal" },
            { accepted: true, actions: [{ kind: "terminate", terminal }] },
          ];
        }
        return [state, { accepted: true, actions: [] }];
      }
      if (event["kind"] === "link_lost") {
        const terminal: CoordinatorTerminal = { reason: "transport_lost" };
        return [
          { ...state, phase: "terminal" },
          { accepted: true, actions: [{ kind: "terminate", terminal }] },
        ];
      }
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

function fakeModelPort(): OnlineMatchModelPort<
  OnlineMatchModel<FakeCoordinatorState, OnlineMatchRequest>
> {
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
// pre-built `state`, so a test keeps a live handle to the driver state
// that `OnlineMatch` otherwise holds privately.
// Never read by either of the two cases below that use `fakeMatchDriver`
// (`fakeMatchScreen` never calls `frame`/`roster`/`tick`/`dispose` -- see
// its own header) -- minimal, fixed stand-ins so `MatchDriverPort`'s
// render-facing quartet (added for `match.ts`'s ONLINE construction mode;
// see that file's own header) is still satisfied.
const FAKE_ONLINE_FRAME: RenderFrame = {
  hud: { finished: false, controlled_owns_ball: false, home_score: 0, away_score: 0, time_left: 0 },
  possession: {},
};
const FAKE_ONLINE_ROSTER: RenderFrameRoster = {};

function fakeMatchDriver(
  state: FakeDriverState,
  live: Readonly<Record<string, string>>,
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
    frame: () => FAKE_ONLINE_FRAME,
    roster: () => FAKE_ONLINE_ROSTER,
    tick: (d) => d.tick,
    dispose: () => {},
  };
}

function fakeMatchPresentation(): MatchPresentationPort<
  Record<string, never>,
  FakeDriverState,
  FakeBatch,
  FakeBatch
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

// A single-frame passthrough, for the one case below that needs a control
// wire to actually reassemble: real chunking/reassembly is pinned for real
// against `@gc/online`'s own module in that package's `lobby_link.spec.ts`
// (this file's header explains why `@gc/screens` cannot import it directly),
// and this case's wire is small enough to never approach the chunking bound
// anyway.
function passthroughLobbyFraming(): LobbyFramingPort {
  return {
    newBuffer: (): LobbyFrameBuffer => ({}),
    absorb: (_buffer, payload) => [payload, undefined] as const,
  };
}

// A link whose star reports one queued control wire *and* one queued
// connection-close event, both already pending on the very first poll -- the
// same-batch race #612 traced. `drainControl()` only polls the star once the
// driver has gone non-active (`fakeLink()`'s own comment above), so a case
// using this constructs its driver already `"failed"`.
function raceLink(peerId: string, wire: string): MatchLobbyLinkPort {
  let batchesPolled = 0;
  let eventsPolled = 0;
  return {
    star: {
      pollBatch: () => {
        if (batchesPolled > 0) {
          return [];
        }
        batchesPolled += 1;
        return [{ channel: "control", peer_id: peerId, message: { payload: wire } }];
      },
      pollEvent: () => {
        if (eventsPolled > 0) {
          return undefined;
        }
        eventsPolled += 1;
        return { kind: "peer_state", peer_id: peerId, state: "closed" };
      },
    },
    send: () => {},
    apply: () => {},
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

// Stands in for `match.ts`'s real fixed-clock `MatchScreen`, which only
// implements the (base/rollback-lab) rollback-consumption seam this
// milestone -- see `real_match.ts`'s header -- and has no online-driven
// mode at all yet: it has no construction path that takes a
// `driverSource()`-shaped port and no code that reads a `controlledPlayer`
// callback to set `state.controlled`. A fake that did either here would be
// asserting against this file's own invention rather than real production
// wiring -- see the two `it.skip`s below this describe block for exactly
// what that gap still blocks. One simulated tick per
// `TICK_SECONDS` of accumulated `dt`, calling the injected rollback source
// exactly the way the real fixed clock would -- the same role
// `match_screen.spec.ts`'s `FakeSimHost` plays for `SimHostPort`.
function fakeMatchScreen(
  state: OnlineMatchState,
  rollbackSource: () => unknown,
): RealMatchScreenPort<OnlineMatchState, unknown> {
  const source = rollbackSource() as RollbackSourceLike;
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

function buildHost(): {
  readonly match: HostOnlineMatch;
  readonly driver: FakeDriverState;
  readonly actions: { readonly go: string }[];
} {
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
  const match = new OnlineMatch(options, fakeModelPort(), (request, coordinator) =>
    newOnlineMatchModel(request, coordinator),
  );
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
    // #489: match_snapshot::COMBAT_VERSION 13 -> 14, and #490: 14 -> 15
    // (see gc-netcode's protocol_conformance::GOLDEN doc comment for the
    // root cause). A real coordinator's protocol::validate_manifest rejects
    // a manifest whose snapshot_version disagrees with the compiled build's,
    // so a stale value here does not merely mismatch a golden byte -- it
    // stalls the whole session, which is why "carries a 1v1 session from the
    // lobby to an agreed result" never reached `ended` regardless of step
    // budget.
    snapshot_version: 15,
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
function realAssignments1v1(
  manifest: ReturnType<typeof realManifest1v1>,
  hostPeerId: string,
  guestPeerId?: string,
) {
  return manifest.slots.map((slot, index) => {
    const isHome = index < 4;
    if (isHome) {
      return {
        producer_kind: "peer",
        producer_id: hostPeerId,
        team: slot.team,
        slot: slot.slot,
        player_id: slot.player_id,
      };
    }
    if (guestPeerId !== undefined) {
      return {
        producer_kind: "peer",
        producer_id: guestPeerId,
        team: slot.team,
        slot: slot.slot,
        player_id: slot.player_id,
      };
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
    return {
      kind: "send",
      message: raw["wire"],
      targets: raw["targets"],
    } as unknown as CoordinatorAction;
  }
  if (kind === "close") {
    return { kind: "close", link_id: raw["link_id"] } as unknown as CoordinatorAction;
  }
  if (kind === "terminate") {
    const terminal = raw["terminal"] as Record<string, unknown>;
    return {
      kind: "terminate",
      terminal: { reason: terminal["reason"], detail: terminal["detail"] },
    } as unknown as CoordinatorAction;
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
  const parsed = JSON.parse(json) as {
    readonly outcomes: readonly { readonly actions: readonly Record<string, unknown>[] }[];
  };
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
  fromGuest: readonly string[],
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
  const handle = new host.Coordinator(
    "host",
    sessionId,
    "host",
    undefined,
    undefined,
    JSON.stringify(REAL_RUNTIME_WIRE),
    REAL_BUILD_ID,
    undefined,
  );
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
function pairedRunning(sessionId: string): {
  readonly host: RealOnlineCoordState;
  readonly guest: RealOnlineCoordState;
} {
  const wasm = loadSimHost();
  const manifest = realManifest1v1(sessionId);
  const hostHandle = new wasm.Coordinator(
    "host",
    sessionId,
    "host",
    undefined,
    undefined,
    JSON.stringify(REAL_RUNTIME_WIRE),
    REAL_BUILD_ID,
    undefined,
  );
  const guestHandle = new wasm.Coordinator(
    "guest",
    sessionId,
    GUEST_PEER_ID,
    "host",
    HOST_LINK_ON_GUEST,
    JSON.stringify(REAL_RUNTIME_WIRE),
    REAL_BUILD_ID,
    JSON.stringify(realExpectation1v1(sessionId)),
  );
  pump(
    hostHandle,
    guestHandle,
    HOST_LINK_ON_GUEST,
    GUEST_LINK_ON_HOST,
    [],
    wiresFrom(guestHandle.connect()),
  );
  pump(
    hostHandle,
    guestHandle,
    HOST_LINK_ON_GUEST,
    GUEST_LINK_ON_HOST,
    wiresFrom(hostHandle.proposeManifest(JSON.stringify(manifest))),
    [],
  );
  pump(
    hostHandle,
    guestHandle,
    HOST_LINK_ON_GUEST,
    GUEST_LINK_ON_HOST,
    wiresFrom(
      hostHandle.assignSlots(
        JSON.stringify(realAssignments1v1(manifest, "host", GUEST_PEER_ID)),
        false,
      ),
    ),
    [],
  );
  pump(
    hostHandle,
    guestHandle,
    HOST_LINK_ON_GUEST,
    GUEST_LINK_ON_HOST,
    wiresFrom(hostHandle.setReady(true)),
    [],
  );
  pump(
    hostHandle,
    guestHandle,
    HOST_LINK_ON_GUEST,
    GUEST_LINK_ON_HOST,
    [],
    wiresFrom(guestHandle.setReady(true)),
  );
  pump(
    hostHandle,
    guestHandle,
    HOST_LINK_ON_GUEST,
    GUEST_LINK_ON_HOST,
    wiresFrom(hostHandle.beginCountdown("countdown.1", 0, 0)),
    [],
  );
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
            undefined,
            event["detail"] as string | undefined,
          );
          break;
        case "link_lost":
          // A transport-reported loss is network-originated -- queued, not
          // applied immediately (the same queue/drain seam as "control";
          // see `net_inbox.rs`'s doc).
          handle.enqueueLinkLost(event["link_id"] as string, event["code"] as string | undefined);
          json = handle.tick();
          break;
        case "abort":
          json = handle.abort(
            event["code"] as string | undefined,
            event["detail"] as string | undefined,
          );
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
            `realCoordinatorPort: unhandled event kind '${String(event["kind"])}' -- this fixture only wires what the real-coordinator flow cases exercise`,
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

function realOnlineModelPort(): OnlineMatchModelPort<
  OnlineMatchModel<RealOnlineCoordState, OnlineMatchRequest>
> {
  return {
    command: (model, event: OnlineMatchDispatchEvent) =>
      command(model, realOnlineModelPorts(), event),
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
  matchDriverFor?: (
    driver: FakeDriverState,
    live: Readonly<Record<string, string>>,
  ) => MatchDriverPort<FakeDriverState, FakeBatch, unknown, FakeCheckpoint>,
): {
  readonly match: RealHostOnlineMatch;
  readonly driver: FakeDriverState;
  readonly actions: { readonly go: string }[];
} {
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
    matchDriver: (matchDriverFor ?? fakeMatchDriver)(driver, {
      [request.peer_id]: request.live ?? "",
    }),
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
  const match = new OnlineMatch(options, realOnlineModelPort(), (req, coord) =>
    newOnlineMatchModel(req, coord),
  );
  return { match, driver, actions };
}

// `fakeMatchDriver` above never reports a checkpoint (the two already-passing
// cases don't need one) -- the real coordinator's `matchFinish` refuses an
// empty final hash (`online_match.ts`'s `lastCheckpointHash` doc), so a case
// that reaches a real agreed result needs its driver to report one, exactly
// once, the way a real driver reports its boundary hash as it runs.
function fakeMatchDriverWithHash(
  driver: FakeDriverState,
  live: Readonly<Record<string, string>>,
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

// ---------------------------------------------------------------------------
// Real `MatchDriverPort`/`MatchSessionPort` fixtures -- for the two control-
// ownership cases below ("keeps control inside the frozen owned set...",
// "makes switching inert in 4v4..."). Unlike `fakeMatchDriver` above (a
// message-relay/terminal-flip bookkeeping fake, never exercising real
// slot-assignment/switch logic at all), these drive a REAL
// `@gc/wasm` `MatchDriverBridge` (`crates/gc-wasm/src/match_driver_bridge.rs`)
// over `gc_netcode::match_driver`'s actual switch rule -- the gap this
// file's header describes ("a spec-local fake that did so would be
// exactly the 'asserting against your own fake' this suite must not do").
// Built from `matchDriverFixtureFreezeJson`/`matchDriverFixtureManifestJson`
// (`crates/gc-wasm/src/match_driver_fixture_bridge.rs`) rather than a
// hand-authored freeze/manifest pair -- see `match_driver_fixture.spec.ts`
// (`@gc/wasm`) for what that fixture proves about itself.
//
// `MatchDriverBridge` has no roster export of its own (confirmed absent
// from `types.ts`'s `MatchDriverBridge` interface and `crates/gc-wasm/src/
// match_driver_bridge.rs`'s `#[wasm_bindgen]` surface -- unlike `SimSession`,
// which has `rosterNumeric`/`rosterIdsAndNames`). `rosterFromManifest` below
// derives match-constant roster identity from the same real fixture
// manifest the bridge itself was constructed from (keeper first, then four
// outfield, per team, home then away -- `gc_render::frame_buffer::roster`'s
// own convention, cross-checked empirically against a decoded frame's
// `hud.controlled_team` during this file's own development) -- real,
// wasm-fixture-sourced data, not a test invention. This is a genuine
// `@gc/wasm`/`crates/gc-wasm` gap worth reporting, not a shortcut.
// ---------------------------------------------------------------------------

interface RealOnlineDriver {
  readonly bridge: MatchDriverBridge;
  readonly session: SimSession;
  readonly roster: RenderFrameRoster;
  // `MatchDriverBridge` has no tick-count getter of its own (confirmed
  // absent from both `types.ts`'s hand-written interface and the compiled
  // artifact's actual `.d.cts` -- unlike `SimSession.inputTick`) -- tracked
  // here instead, incremented once per `advance()` call.
  tickCount: number;
}

interface RealOnlineCheckpoint {
  readonly tick: number;
  readonly hash: string;
}

interface RealOnlineBatch {
  readonly control: readonly unknown[];
  readonly checkpoints: readonly RealOnlineCheckpoint[];
  readonly live: Readonly<Record<string, string>>;
}

function rosterFromManifest(manifestJson: string): RenderFrameRoster {
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

function realMatchDriverPort(
  wasm: SimHost,
): MatchDriverPort<RealOnlineDriver, RealOnlineBatch, unknown, RealOnlineCheckpoint> {
  return {
    create(options) {
      const freezeJson = options.freeze as string;
      const manifestJson = options.manifest as string;
      const session = new wasm.Session("nebula", "orion", 7, 20, 3);
      const bridge = new wasm.MatchDriverBridge(
        session,
        options.role,
        options.peer_id,
        freezeJson,
        manifestJson,
        undefined,
      );
      bridge.initializeTransport();
      return { bridge, session, roster: rosterFromManifest(manifestJson), tickCount: 0 };
    },
    status: (d) => JSON.parse(d.bridge.statusJson()) as string,
    advance: (d, sample) => {
      const s = sample as InputSample;
      const wire = wasm.inputFrameNewSample(s.move_x, s.move_y, s.held, s.edges);
      const batch = JSON.parse(d.bridge.advance(wire)) as RealOnlineBatch;
      d.tickCount += 1;
      return batch;
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
    // Not exercised by either case below (both stay "active" throughout) --
    // a best-effort field mapping so this port's required shape is still
    // satisfied.
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
    batchControl: (b) => b.control,
    batchCheckpoints: (b) => b.checkpoints,
    batchLive: (b) => b.live,
    frame: (d) => frameBuffer.decode(wasm.buildMatchDriverRenderFrame(d.bridge, 0, 0)),
    roster: (d) => d.roster,
    tick: (d) => d.tickCount,
    dispose: (d) => {
      d.session.free();
    },
  };
}

// `fakeMatchPresentation`'s own behavior (`consume` is a straight
// passthrough) never actually touches `TDriver`/`TBatch`, but its
// declared type is hardcoded to `FakeDriverState`/`FakeBatch` -- this is
// the same passthrough, generic over the real driver/batch types instead.
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

// `gc_netcode::match_driver`'s own slot-assignment: `live` carries the
// SLOT string (e.g. "home_2"), and `MatchSessionPort.playerIndex` maps it to
// an index into `OnlineMatchState.players` -- looked up by player id via the
// same fixture manifest's `slots` list, not by assuming any array-order
// convention (independent of `rosterFromManifest`'s own ordering choice
// above).
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

function fakeKeyboard(down: Readonly<Record<string, boolean>>): KeyboardState {
  return {
    isDown: (...keys: readonly string[]): boolean => keys.some((key) => down[key] === true),
  };
}

type RealDriverOnlineMatch = OnlineMatch<
  RealOnlineDriver,
  RealOnlineBatch,
  unknown,
  RealOnlineCheckpoint,
  Record<string, never>,
  RealOnlineBatch,
  FakeCoordinatorState,
  OnlineMatchModel<FakeCoordinatorState, OnlineMatchRequest>
>;

// Builds a solo, bot-filled host over a REAL `MatchDriverBridge`, driven
// through a REAL `MatchScreen` constructed in ONLINE mode
// (`MatchScreenAsOnlineMatchScreen` -- match.ts) -- not `fakeMatchScreen`.
// This is what makes the two cases below prove `match.ts`'s real online
// construction mode, not a spec-local reimplementation of it. The
// coordinator/model stays the FAKE bookkeeping one (`fakeModelPort`) --
// neither case reaches a terminal/result, so the model layer is not what is
// under test here.
function buildRealDriverHost(
  mode: "1v1" | "4v4",
  request: OnlineMatchRequest,
  renderer: RenderPort = { draw: () => {} },
): RealDriverOnlineMatch {
  const wasm = loadSimHost();
  const freezeJson = wasm.matchDriverFixtureFreezeJson(mode, 0, 1);
  const manifestJson = wasm.matchDriverFixtureManifestJson(mode);
  const options: OnlineMatchOptions<
    RealOnlineDriver,
    RealOnlineBatch,
    unknown,
    RealOnlineCheckpoint,
    Record<string, never>,
    RealOnlineBatch,
    FakeCoordinatorState
  > = {
    request: { ...request, freeze: freezeJson, manifest: manifestJson },
    coordinator: fakeCoordinatorState("host"),
    link: fakeLink(),
    matchDriver: realMatchDriverPort(wasm),
    matchPresentation: identityMatchPresentation<RealOnlineDriver, RealOnlineBatch>(),
    matchSession: realMatchSessionPort(manifestJson),
    lobbyFraming: fakeLobbyFraming(),
    contract: fakeContract(),
    observer: fakeObserver(),
    newMatch: (opts) =>
      new MatchScreenAsOnlineMatchScreen(
        new MatchScreen(
          {
            createHost: () => {
              throw new Error("unused: online mode never calls createHost");
            },
            renderer,
            keyboard: fakeKeyboard({}),
          },
          {
            online: opts.rollbackSource(),
            ...(opts.combatEnabled !== undefined ? { combat_enabled: opts.combatEnabled } : {}),
          },
        ),
      ),
    onAction: () => {},
  };
  return new OnlineMatch(options, fakeModelPort(), (req, coord) => newOnlineMatchModel(req, coord));
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
    const { match, driver } = buildRealHost(
      coordinator,
      HOST_REQUEST,
      fakeLink(),
      fakeMatchDriverWithHash,
    );
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

  // Which slot is LIVE is
  // `gc_netcode::match_driver`'s real switch-rule output, reached here
  // through a REAL `MatchDriverBridge` (`realMatchDriverPort`, above -- not
  // `fakeMatchDriver`'s spec-invented `live` map) driven by a REAL
  // `MatchScreen` in `match.ts`'s new ONLINE construction mode
  // (`MatchScreenAsOnlineMatchScreen`, not `fakeMatchScreen`). "1v1" mode
  // with one connected human seats the WHOLE home outfield block to this
  // peer (`owned`: home_1..home_4 -- `zyro_vex`/`mika_olu`/`rok_tann`/
  // `sela_dwin`; see `realMatchDriverPort`'s header and
  // `match_driver_fixture.spec.ts`'s "freezeJson reports a contiguous owned
  // block per human"), explicitly excluding both keepers (`ozzo`/`gax_oru`,
  // never in `owned` at all -- the protocol's slot list has no keeper
  // entries). A "k" key press every tick both drives real switch edges
  // through the real input-sample pipeline (`MatchScreen`'s contextual
  // input state machine -- match.ts) and proves the frozen-set/keeper
  // exclusion holds under sustained switching pressure, not just once.
  it("keeps control inside the frozen owned set and off both keepers", () => {
    const request: OnlineMatchRequest = {
      ...HOST_REQUEST,
      mode: "1v1",
      live: "home_1",
      owned: ["home_1", "home_2", "home_3", "home_4"],
    };
    const match = buildRealDriverHost("1v1", request);
    const owned = new Set(["zyro_vex", "mika_olu", "rok_tann", "sela_dwin"]);
    const seen = new Set<string>();

    for (let i = 0; i < 90; i += 1) {
      match.event({ kind: "key", key: "k" });
      match.update(TICK_SECONDS);
      const state = match.match.state;
      const player = state.players[state.controlled];
      expect(
        player,
        `tick ${i}: controlled index ${state.controlled} must name a real player`,
      ).toBeDefined();
      if (player !== undefined) {
        expect(
          owned.has(player.id),
          `tick ${i}: controlled player '${player.id}' left the frozen owned set`,
        ).toBe(true);
        expect(player.id, `tick ${i}: controlled player must never be a keeper`).not.toBe("ozzo");
        expect(player.id, `tick ${i}: controlled player must never be a keeper`).not.toBe(
          "gax_oru",
        );
        seen.add(player.id);
      }
    }
    // Sustained switching pressure over 90 ticks must have actually moved
    // control at least once within the owned set -- otherwise this proves
    // nothing beyond "the initial slot happens to be legal".
    expect(
      seen.size,
      "sustained switching pressure never actually switched control",
    ).toBeGreaterThan(1);
  });

  // Same real driver/screen wiring as
  // above, over a "4v4" fixture with one connected human -- `slots_per_human`
  // is 1 in that mode, so this peer's `owned` set is the SINGLETON
  // `["home_1"]` (`zyro_vex`). Proving every switch attempt still resolves
  // to that one slot -- not a crash, not `undefined`, not a different
  // player -- is exactly "inert... without branching on the mode": nothing
  // in `gc_netcode::match_driver`'s switch rule, `driverSource()`, or
  // `match.ts`'s new consumption of it special-cases a singleton owned set.
  it("makes switching inert in 4v4 without branching on the mode", () => {
    const request: OnlineMatchRequest = {
      ...HOST_REQUEST,
      mode: "4v4",
      live: "home_1",
      owned: ["home_1"],
    };
    const match = buildRealDriverHost("4v4", request);

    for (let i = 0; i < 60; i += 1) {
      match.event({ kind: "key", key: "k" });
      match.update(TICK_SECONDS);
      const state = match.match.state;
      const player = state.players[state.controlled];
      expect(player?.id, `tick ${i}: the singleton owned slot must stay controlled`).toBe(
        "zyro_vex",
      );
    }
  });

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
    const { match: hostMatch } = buildRealHost(
      hostCoordinator,
      HOST_REQUEST,
      relayLink(guestHandle, HOST_LINK_ON_GUEST),
    );
    const { match: guestMatch } = buildRealHost(
      guestCoordinator,
      guestRequest,
      relayLink(hostHandle, GUEST_LINK_ON_HOST),
    );
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

  it("lets a same-poll Abort beat the generic link-lost verdict (#612)", () => {
    const driver: FakeDriverState = { status: "failed", tick: 0 };
    const initialState: OnlineMatchState = {
      time_left: 600,
      score: { home: 0, away: 0 },
      controlled: 0,
      players: [{ id: "zyro_vex", team: "home", pos: new Vec2(0, 0), facing: new Vec2(0, 1) }],
    };
    const abortWire = JSON.stringify({ kind: "abort", reason: "peer_abort" });
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
      link: raceLink("peer", abortWire),
      matchDriver: fakeMatchDriver(driver, { [HOST_REQUEST.peer_id]: HOST_REQUEST.live ?? "" }),
      matchPresentation: fakeMatchPresentation(),
      matchSession: fakeMatchSession(),
      lobbyFraming: passthroughLobbyFraming(),
      contract: fakeContract(),
      observer: fakeObserver(),
      newMatch: (opts) => fakeMatchScreen(initialState, opts.rollbackSource),
    };
    const modelPort: OnlineMatchModelPort<
      OnlineMatchModel<FakeCoordinatorState, OnlineMatchRequest>
    > = {
      command: (model, event) =>
        command(model, { ...modelPorts(), coordinator: abortAwareCoordinatorPort() }, event),
      ended,
      exitRoute,
      ABORT_PROMPT,
    };
    const match = new OnlineMatch(options, modelPort, (request, coordinator) =>
      newOnlineMatchModel(request, coordinator),
    );

    // One `update()` is one `drainControl()` call: both the Abort and the
    // close are already queued before it runs.
    match.update(TICK_SECONDS);

    expect(ended(match.model)).toBe(true);
    const model = match.model as { readonly terminal?: { readonly reason: string } };
    expect(model.terminal?.reason).toBe("peer_abort");
  });

  // `overlayLines` reads `this.match.state` and diagnostics off the
  // injected ports directly and formats them into plain lines -- no
  // `@gc/render` involved (this screen's own overlay is raw text this
  // milestone; see `online_match.ts`'s header). Passing `combatState: null`
  // from the fake match screen takes `combat.model`'s early
  // `{ enabled: false }` return, so this only proves the overlay's own
  // labels, not any particular combat readout.
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

// This case stays genuinely blocked, against the two real cases above
// (`realMatchDriverPort`, driving an actual `MatchDriverBridge`, and
// `match.ts`'s ONLINE `MatchScreen` construction mode/`drawOnline`), for a
// narrower and more specific reason than a missing dependency: `@gc/render`
// is a declared dependency. `crates/gc-wasm/src/session.rs`'s
// `Session::new`/`Session::step` has a real `combat_enabled` parameter
// (`match_screen.spec.ts`'s "constructs and drives combat only behind the
// explicit option" exercises it) -- but that is `SimSession`'s own combat
// companion, a SEPARATE thing from `MatchDriverBridge` (this file's
// `realMatchDriverPort`), which still takes no `combat_enabled` of its own
// and, per `crates/gc-wasm/src/match_driver_bridge.rs`, only ever carries a
// combat companion if its `initialSnapshotOverride` already has one (e.g.
// `OnlineCombatPhasesBridge.onlineCombatPhaseBoundaryZero`'s seven pinned
// snapshots -- not this fixture's own default boundary zero, which the
// renderer-smoke case above confirms decodes `combatPresent: false`). Even
// granting a combat-bearing snapshot, the deeper blocker still holds:
// `gc_render::frame_buffer`'s own module doc states outright that
// `RenderFrame::combat` (the telegraph model with `Vec2` positions) is
// NEVER encoded onto the flat wire -- only a `combat_present` boolean
// crosses (confirmed by reading, not editing, `crates/gc-render/src/
// frame_buffer.rs`) -- so per-tick combat readiness/telegraph timing has no
// path to TypeScript at all yet, wasm-bound snapshot or not. Reproducing
// "every quiet frame past kickoff read ready, and committing zero times
// there proves nothing" with a fake would still mean re-implementing
// `sim.combat`'s request-rejection gates, not just relaying messages -- so
// this stays skipped, for the wire-format reason above rather than a
// missing-binding one.
describe.skip("online combat families [needs sim.combat's real readiness/telegraph timing (Rust-owned, crates/gc-sim, no wasm bridge)]", () => {
  it.skip("drives and shows every accepted family from local keyboard and gamepad", () => {});
});

// "online match app routing" / "routes the lobby's synchronized start into
// the online match" moved to packages/app/src/online_match_flow.spec.ts:
// `@gc/app` depends on `@gc/screens`, never the other way around (its own
// `package.json`; ARCHITECTURE.md's directory table: `@gc/app` is the layer
// meant to wire screens together), and the module the case is actually
// about -- `App.start_online_match` -- lives there. It passes in its new
// home: moving it also surfaced that `App`'s `start_online_match` was an
// unconditional stub that never read the mounted lobby's state at all, not
// merely a case with nothing to exercise it -- see that file's header for
// the fix and what the moved case can and cannot prove from this side of
// the boundary. Do not re-add it here.

// `@gc/render`'s `pitch`/`matchHud` drawing is reachable here (a declared
// dependency, and its pure `pitchDrawCommands`/`matchHudCommands` path
// needs no WebGL context -- see `packages/render/src/pitch.spec.ts`'s own
// pattern), but a hand-built `RenderFrame` fixture would only reprove
// `@gc/render`'s own pure path, which `pitch.spec.ts`/`match_hud.spec.ts`
// already cover in that package. This case's whole point instead rests on
// the translation from a *live online frame*:
// `gc_render::frame_buffer::encode` read back via `@gc/wasm`'s
// `buildRenderFrame` off a real `SimSession`, wired into a drawable frame
// by `match.ts`'s `draw()`.
// `MatchDriverBridge.renderFrameBuild`/`SimHost.buildMatchDriverRenderFrame`
// (`@gc/wasm`) provide that, and `match.ts`'s ONLINE construction mode
// (`MatchScreen.drawOnline`) turns a live `MatchDriverBridge` frame into a
// corrected, drawable `RenderFrame` -- see `realMatchDriverPort`'s header
// (above) for the same real driver wiring the two control-ownership cases
// already exercise, reused here for drawing instead.
//
// `MatchDriverBridge` still has no roster export of its own (the same real
// `@gc/wasm`/`crates/gc-wasm` gap `realMatchDriverPort`'s header names) --
// `species_shape`/`species_color`/`radius`/`is_keeper` are therefore NOT
// real wasm-sourced data for this fixture (`rosterFromManifest` only
// derives `ids`/`teams`, all it can honestly derive from the real fixture
// manifest). So this case does not feed the captured frame into
// `@gc/render`'s `pitchDrawCommands` -- doing so would mean fabricating
// exactly the roster fields that gap leaves missing, the "asserting
// against your own fixture" this suite must not do. What IS proved here
// is real: a genuinely advancing `MatchDriverBridge`'s per-tick frame,
// decoded through `@gc/render`'s real `frame_buffer.decode`
// (`realMatchDriverPort.frame`), reaching `RenderPort.draw` with
// `hud.controlled`/`control.controlled`/the per-slot `players.controlled`
// highlight corrected to the driver's real assignment -- not the raw sim's
// own, uncorrected notion -- see `match.ts`'s `drawOnline`.
describe("online match renderer smoke", () => {
  it("draws a live online frame with its combat model and HUD", () => {
    const request: OnlineMatchRequest = {
      ...HOST_REQUEST,
      mode: "1v1",
      live: "home_1",
      owned: ["home_1", "home_2", "home_3", "home_4"],
    };
    const drawn: { readonly frame: RenderFrame; readonly roster: RenderFrameRoster }[] = [];
    const renderer: RenderPort = {
      draw: (frame, roster) => {
        drawn.push({ frame, roster });
      },
    };
    const match = buildRealDriverHost("1v1", request, renderer);

    // A few ticks of real switching pressure, so this is a genuinely live,
    // advancing frame -- not merely the kickoff snapshot.
    for (let i = 0; i < 10; i += 1) {
      match.event({ kind: "key", key: "k" });
      match.update(TICK_SECONDS);
    }
    match.draw();

    expect(drawn.length, "draw() must reach RenderPort.draw").toBe(1);
    const { frame, roster } = drawn[0]!;
    // Real, wasm-decoded data: a full 5v5 roster, and the clock has
    // genuinely ticked down from the fixture's `matchDriverFixtureInitialSnapshot`
    // default (20 seconds).
    expect(roster.ids?.length, "a real 5v5 roster crossed the wire").toBe(10);
    expect(frame.hud.time_left).toBeLessThan(20);
    // `combatPresent` is a real, decoded field off the wire header (`gc_render
    // ::frame_buffer`'s `combat_present` word) -- `false` here is itself the
    // honest report: this fixture's boundary-zero snapshot carries no combat
    // companion (`matchDriverFixtureInitialSnapshot`'s default), not a stand-in
    // value this test invented.
    expect((frame as unknown as { readonly combatPresent: boolean }).combatPresent).toBe(false);

    // The corrected control: matches `match.match.state.controlled` (the
    // same driver-corrected value the two "keeps control..."/"makes
    // switching..." cases already assert on), one-based on the wire.
    const state = match.match.state;
    expect(frame.hud.controlled).toBe(state.controlled + 1);
    expect(frame.control?.controlled).toBe(state.controlled + 1);
    expect(frame.players?.controlled[state.controlled]).toBe(true);
    const otherHighlighted = (frame.players?.controlled ?? []).filter(
      (flag, index) => flag && index !== state.controlled,
    );
    expect(otherHighlighted, "exactly one player is highlighted as controlled").toEqual([]);
  });
});
