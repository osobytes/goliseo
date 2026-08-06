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
// Two things changed since this file was first ported as a wall of
// `it.skip`, and one did not:
//
// - `@gc/input` IS now a declared dependency of this package
//   (`package.json`) -- that half of the original blocker note was stale.
// - `@gc/wasm` now exports a real `Coordinator`/`MatchDriverBridge`
//   (`crates/gc-wasm/src/coordinator_bridge.rs`), so "no wasm bridge this
//   milestone" is stale too, as a general claim.
// - `@gc/render` is still not a declared dependency of `@gc/screens`, and
//   this port may not edit `package.json` (another agent owns manifests
//   this batch). That is a real, current blocker for anything that needs
//   the real pitch/HUD renderer.
//
// `online_match.ts` is entirely port-injected (`MatchDriverPort`,
// `MatchPresentationPort`, `MatchSessionPort`, `MatchLobbyLinkPort`,
// `LobbyFramingPort`, `MatchContractPort`, an observer port, and a
// `newMatch` factory returning `real_match.ts`'s `RealMatchScreenPort`) --
// following the pattern `match_screen.spec.ts` already established for
// `match.ts`/`SimHostPort`, hand-written fakes standing in for those ports
// let two of the six "online match screen flow" cases run for real against
// `online_match.ts`'s and `online_match_model.ts`'s own logic, with no
// `@gc/wasm` or `@gc/render` dependency at all. Session/slot-assignment
// protocol logic (which slot is LIVE, which coordinator reason a peer's
// abort produces on the *other* peer) is Rust-owned and not reimplemented
// here -- see each remaining `it.skip`'s own comment for why that specific
// case still can't run.

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

describe("online match screen flow", () => {
  // Ported as `it.skip`: reaching an *agreed* full-time result needs the
  // real coordinator to flip to a `"completed"` terminal once every peer's
  // boundary hash has been acknowledged -- deliberately not modeled by the
  // fake `CoordinatorPort` above (see this file's header and
  // `online_match_model.spec.ts`'s own header: "the coordinator itself is
  // only ever asked to relay a message or flip to terminal, never to
  // arbitrate admission"). Faking that admission would mean
  // re-implementing `coordinator.lua`'s acknowledgement protocol, exactly
  // what v2/README.md §2.1 exists to prevent. `@gc/wasm`'s `Coordinator`
  // does this for real now, but `@gc/screens` has no dependency edge onto
  // `@gc/wasm` this batch (absent from `package.json`).
  it.skip("carries a 1v1 session from the lobby to an agreed result", () => {});

  // Ported as `it.skip`: which slot is LIVE is the real coordinator's
  // slot-assignment/switch-rule output (`match_driver.materialize_authored`
  // in the Lua original), and `match.state.controlled` is derived from it
  // by the real `match.ts` (owned by another agent this batch; `match.ts`
  // itself still only ports its rollback-consumption seam, per its own
  // header). A fake `MatchDriverPort.batchLive` and a fake match screen
  // could return whatever this test wants them to, which would make the
  // assertion ("control never left the frozen owned set") true of the
  // fixture rather than of the real switch algorithm -- exactly the
  // "reproduces behaviour, not intent" line this port must not cross.
  it.skip("keeps control inside the frozen owned set and off both keepers", () => {});

  // Ported as `it.skip`: same unblocker as above -- a singleton owned set
  // making every switch branch return the live slot is a property of the
  // real switch rule, not of this screen's own logic.
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

  // Ported as `it.skip`: a deliberate abort is provably local
  // (`local_abort`, exercised by `online_match_model.spec.ts`'s "aborts on
  // a second pause request" case for real). What only this flow spec can
  // prove is the *other* peer's side -- that receiving the relayed abort
  // produces `terminal.reason === "peer_abort"` on the guest -- and that
  // mapping is the real coordinator's own protocol decision on an inbound
  // wire message, not something `online_match_model.ts` computes locally
  // (`"peer_abort"` does not appear anywhere in that module). A fake that
  // invented that mapping would be asserting the fake's own design, not
  // `coordinator.lua`'s.
  it.skip("aborts deliberately and ends the session for every peer", () => {});

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

// `@gc/render` (pitch/HUD drawing) is not a declared dependency of
// `@gc/screens`, and telegraph/readiness/kickoff-hold timing is `sim`'s
// real physics (Rust-owned) -- reproducing "every quiet frame past kickoff
// read ready, and committing zero times there proves nothing" with a fake
// would mean re-implementing `sim.combat`'s request-rejection gates, not
// just relaying messages. `@gc/input` being a declared dependency now (the
// earlier blocker note was stale on that point) doesn't change either of
// those two, so this stays skipped.
describe.skip("online combat families [needs @gc/render (not a declared dependency) and sim.combat's real readiness/telegraph timing (Rust-owned, crates/gc-sim)]", () => {
  it.skip("drives and shows every accepted family from local keyboard and gamepad", () => {});
});

// `@gc/app` depends on `@gc/screens`, never the other way around (its own
// `package.json`; v2/README.md's directory table: `@gc/app` is the layer
// meant to wire screens together). This package's `package.json` (which
// this port may not edit) does not, and architecturally should not, depend
// on `@gc/app` -- the same reasoning `lobby_flow.spec.ts`'s "is reachable
// from the title and returns to it" is skipped under.
describe.skip("online match app routing [@gc/app depends on @gc/screens, not the reverse -- see this file's header]", () => {
  it.skip("routes the lobby's synchronized start into the online match", () => {});
});

// `@gc/render`'s `pitch.draw`/`match_hud_render.draw` are not a declared
// dependency of `@gc/screens` (this file's header) -- unlike
// `overlayLines` above, this case's whole point is that the *real* pixel
// renderer runs over a live online frame without throwing, which a fake
// `GraphicsBackend` stub (`lobby.spec.ts`'s pattern) cannot stand in for:
// there is no real implementation of that renderer reachable from this
// package to run.
describe.skip("online match renderer smoke [needs @gc/render, not a declared dependency of @gc/screens]", () => {
  it.skip("draws a live online frame with its combat model and HUD", () => {});
});
