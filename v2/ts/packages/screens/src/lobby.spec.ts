// Ported from spec/screens/lobby_spec.lua.
//
// The Lua spec drives the real `game.online.coordinator` (Rust-owned,
// `crates/gc-netcode`; v2/README.md §2.1) through `lobby_model.ts`'s
// injected `CoordinatorPort`. Most of what it asserts turns out to be about
// `lobby.ts`/`lobby_model.ts`'s own layout and control-flow -- role choice,
// signaling digests, invitation effects, focus movement, terminal-reason
// text -- and none of that needs the coordinator to arbitrate admission,
// slot assignment, or pair preferences for real. A small hand-scripted
// `CoordinatorPort` fake (mirroring `match_presentation.spec.ts`'s
// `fakeRollbackEvents`) tracks only phase/manifest/assignment bookkeeping,
// and is enough to run most of the Lua spec's assertions for real.
//
// Three cases genuinely need the coordinator's real slot-assignment and
// pair-preference protocol logic to produce meaningful output (exactly
// which slots read "LIVE" vs "AI (OWNED)" vs "AI FILL", and exactly which
// slots offer a pair control). That logic now exists for real --
// `@gc/wasm`'s `Coordinator` (`crates/gc-wasm/src/coordinator_bridge.rs`)
// is the full reducer, not a fake -- but `@gc/screens` has no dependency
// edge onto `@gc/wasm` (absent from `package.json`, unlinked in
// `node_modules`; this package's `CoordinatorPort` is deliberately an
// injected seam that `@gc/app` wires in production, mirroring
// `sim_host.ts`'s role for `MatchDriverPort`). Reproducing the assignment
// algorithm with a hand-written fake instead -- rather than waiting for
// that dependency edge -- would mean re-implementing `coordinator.lua`'s
// rules a second time, exactly what v2/README.md §2.1 exists to prevent.
// Those three are ported as `it.skip`. A fourth (`renders every state
// without touching a real display`) is unblocked below: `@gc/ui` (with its
// `draw` module and `GraphicsBackend`) *is* a declared dependency of this
// package (`online_lobby.ts` already draws through it), so the stated
// blocker -- "this package does not own that seam" -- was stale.

import { describe, expect, it } from "vitest";
import { draw, hit } from "@gc/ui";
import type { GraphicsBackend, Layout } from "@gc/ui";
import {
  newState,
  layout as lobbyLayout,
  update as lobbyUpdate,
  type LobbyAction,
  type LobbyScreenEvent,
  type LobbyScreenState,
} from "./lobby.ts";
import { view as lobbyModelView } from "./lobby_model.ts";
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
    step(state: CoordinatorState, event: CoordinatorEvent): readonly [CoordinatorState, CoordinatorOutcome] {
      switch (event["kind"]) {
        case "propose_manifest": {
          const manifest = event["manifest"] as SessionManifest;
          return [
            {
              ...state,
              phase: "manifest",
              manifest,
              manifest_id: "manifest-fixture",
              peers: state.peers.map((peer) => ({ ...peer, accepted_manifest_id: "manifest-fixture" })),
            },
            { accepted: true, actions: [] },
          ];
        }
        case "assign_slots": {
          const assignments = event["assignments"] as readonly SessionSlotProducer[];
          return [{ ...state, phase: "assigned", assignments }, { accepted: true, actions: [] }];
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
          return [{ ...state, phase: "countdown", countdown_remaining: event["remaining_ticks"] as number }, { accepted: true, actions: [action] }];
        }
        case "abort": {
          return [{ ...state, phase: "terminal", terminal: { reason: "local_abort" } }, { accepted: true, actions: [] }];
        }
        case "leave": {
          return [{ ...state, phase: "terminal", terminal: { reason: "guest_left" } }, { accepted: true, actions: [] }];
        }
        default:
          return [state, { accepted: true, actions: [] }];
      }
    },
    planAssignments(manifest: SessionManifest, seating: readonly string[]): readonly SessionSlotProducer[] | undefined {
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

function clickOn(currentLayout: Layout, id: string): { readonly kind: "click"; readonly x: number; readonly y: number } {
  const widget = hit.find(currentLayout, id);
  if (!widget?.rect) {
    throw new Error(`missing widget ${id}`);
  }
  return { kind: "click", x: widget.rect.x + widget.rect.w / 2, y: widget.rect.y + widget.rect.h / 2 };
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

// Every method a no-op, matching the Lua original's `with_stub_graphics`:
// real draw code executes against this, so a nil field or a bad projection
// fails here rather than on a device. `getDimensions` answers the same
// 1280x720 the Lua stub did.
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
    push: noop,
    pop: noop,
    translate: noop,
    scale: noop,
    setFont: noop,
  };
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
    for (const slot of ["home_1", "home_2", "home_3", "home_4", "away_1", "away_2", "away_3", "away_4"]) {
      expect(hit.find(currentLayout, `slot_${slot}`)).not.toBeNull();
    }
    const homeKeeper = hit.find(currentLayout, "keeper_home");
    expect(homeKeeper?.text?.includes("PROTECTED AI")).toBe(true);
    expect(hit.find(currentLayout, "keeper_away")).not.toBeNull();
  });

  it("never renders a pasted or exported signaling blob", () => {
    let state = hosting();
    state = dispatch(state, { kind: "lobby", command: { kind: "paste", text: "SDPSECRET".repeat(40) } });
    for (const widget of lobbyLayout(state)) {
      if (widget.text) {
        expect(widget.text.includes("SDPSECRET")).toBe(false);
      }
    }
  });

  it("keeps only a digest of a signaling blob it has handed over", () => {
    let state = hosting();
    state = dispatch(state, { kind: "lobby", command: { kind: "signal", peer_id: "guest_1", signal: "offer:blob" } });
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
    expect(nextState.effects.map((effect) => effect.kind).join(",")).toBe("open_peer,request_offer");
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
    expect((action as LobbyAction | undefined)?.go).toBe("main_menu");
    const [, clicked] = lobbyUpdate(state, clickOn(lobbyLayout(state), "leave"));
    expect((clicked as LobbyAction | undefined)?.go).toBe("main_menu");
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
  // (`graphics_backend.ts`'s header), but the Lua original didn't need one
  // either -- it stubbed `love.graphics` -- so a hand-written no-op
  // `GraphicsBackend` plays the same role here.
  it("renders every state without touching a real display", () => {
    const roleState = newState(VP, ports());
    const hostState = hosting();
    const locked = dispatch(
      dispatch(hosting(), { kind: "lobby", command: { kind: "bot_fill" } }),
      { kind: "lobby", command: { kind: "lock" } },
    );
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

  // Ported as `it.skip`: which slots read LIVE / AI (OWNED) / AI FILL needs
  // the real coordinator's slot-assignment algorithm (team_humans /
  // slots_per_human distribution) -- the fixture `planAssignments` above is
  // deliberately not a faithful port of it (see this file's header). The
  // real algorithm exists now, in `@gc/wasm`'s `Coordinator`; the blocker is
  // that `@gc/screens` has no dependency edge onto `@gc/wasm` this batch
  // (this file's header).
  it.skip("names the AI-driven slots inside a human's owned set", () => {});

  // Ported as `it.skip`: same unblocker -- which slots offer a pair control
  // depends on the real assignment/ownership algorithm.
  it.skip("offers a pair control only where there is a pair to choose", () => {});

  // Ported as `it.skip`: same unblocker.
  it.skip("offers no pair control at all in 1v1 or 4v4", () => {});
});
