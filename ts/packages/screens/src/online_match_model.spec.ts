// This suite exercises `online_match_model.ts`'s pure flow policy. Its
// `CoordinatorState` instances never come from a real, running session: the
// coordinator itself is Rust-owned (`crates/gc-netcode`; ARCHITECTURE.md
// §1.1) with no wasm bridge yet, so there is no way to drive one to
// `running` over real canonical wires here.
//
// That's not a loss for this suite's purpose. Every assertion below is
// about `online_match_model.ts`'s *own* control flow: which coordinator
// events it issues, in what order, and how its local fields (`phase`,
// `score`, `tick`, `notice`, `abort_prompt`, `terminal`, `result`) evolve.
// The coordinator itself is only ever asked to relay a message or flip to
// `terminal`, never to arbitrate admission, slots, or readiness -- so a
// small hand-scripted `CoordinatorPort` fake (in the same spirit as
// `match_presentation.spec.ts`'s `fakeRollbackEvents`) is enough to run
// every test below for real, without reimplementing any of the
// coordinator's own protocol logic.

import { describe, expect, it } from "vitest";
import {
  CONTROLLER_NOTICE,
  FOCUS_NOTICE,
  NOTICE_SECONDS,
  PAUSE_NOTICE,
  command,
  ended,
  exitRoute,
  newOnlineMatchModel,
  type CoordinatorEvent,
  type CoordinatorOutcome,
  type CoordinatorPort,
  type CoordinatorStateCore,
  type CoordinatorTerminal,
  type OnlineMatchEffect,
  type OnlineMatchModel,
  type OnlineMatchModelPorts,
  type OnlineMatchRequestCore,
  type ProtocolMessage,
  type ProtocolPort,
  type RollbackEventStepCore,
  type RollbackWrappedEvent,
  type LifecyclePayload,
} from "./online_match_model.ts";

// ---------------------------------------------------------------------------
// Fake coordinator/protocol: message relay + terminal flip only, no protocol
// acceptance/admission/readiness logic. See this file's header.
// ---------------------------------------------------------------------------

interface FakeCoordinatorState extends CoordinatorStateCore {
  readonly session_id: string;
  readonly local_result?: {
    readonly home_score: number;
    readonly away_score: number;
    readonly final_hash: string;
  };
}

function fakeCoordinator(role: "host" | "guest", hostLinkId?: string): FakeCoordinatorState {
  return {
    phase: "running",
    role,
    session_id: "session/fixture",
    ...(hostLinkId !== undefined ? { host_link_id: hostLinkId } : {}),
  };
}

function terminalReasonFor(event: CoordinatorEvent): string {
  if (event["kind"] === "abort") {
    return "local_abort";
  }
  if (event["kind"] === "link_lost") {
    // `code` is a `TransportErrorCode` (a string union) on this event; read it
    // as one rather than stringifying an `unknown`, which would silently
    // produce "[object Object]" if the shape ever changed.
    const code = event["code"];
    return typeof code === "string" ? code : "transport_lost";
  }
  if (event["kind"] === "netcode_failure") {
    const failure = event["failure"];
    if (failure === "late_input") return "late_input";
    if (failure === "desync") return "hash_mismatch";
    if (failure === "input_channel") return "input_channel_failure";
    return "unknown";
  }
  throw new Error(`fake coordinator cannot terminate on ${String(event["kind"])}`);
}

function fakeCoordinatorPort(): CoordinatorPort<FakeCoordinatorState> {
  return {
    step(state, event): readonly [FakeCoordinatorState, CoordinatorOutcome] {
      switch (event["kind"]) {
        case "abort":
        case "link_lost":
        case "netcode_failure": {
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
        case "match_phase": {
          const outcome: CoordinatorOutcome = {
            accepted: true,
            actions: [
              {
                kind: "send",
                message: { kind: "match_phase", body: { phase: event["phase"] } },
                targets: ["peer"],
              },
            ],
          };
          return [state, outcome];
        }
        case "hash_report": {
          const outcome: CoordinatorOutcome = {
            accepted: true,
            actions: [
              {
                kind: "send",
                message: {
                  kind: "hash_report",
                  body: { tick: event["tick"], boundary_hash: event["boundary_hash"] },
                },
                targets: ["peer"],
              },
            ],
          };
          return [state, outcome];
        }
        case "finish": {
          if (state.role === "host") {
            const outcome: CoordinatorOutcome = {
              accepted: true,
              actions: [
                {
                  kind: "send",
                  message: {
                    kind: "result_ack",
                    body: { home_score: event["home_score"], away_score: event["away_score"] },
                  },
                  targets: ["peer"],
                },
              ],
            };
            return [state, outcome];
          }
          const localResult = {
            home_score: event["home_score"] as number,
            away_score: event["away_score"] as number,
            final_hash: event["final_hash"] as string,
          };
          return [
            { ...state, local_result: localResult },
            { accepted: true, actions: [] },
          ];
        }
        default:
          return [state, { accepted: true, actions: [] }];
      }
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

function ports(): OnlineMatchModelPorts<FakeCoordinatorState> {
  return {
    coordinator: fakeCoordinatorPort(),
    protocol: fakeProtocolPort(),
    localResult: () => undefined,
    isCompleted: (terminal) => terminal.reason === "completed",
  };
}

interface FixtureRequest extends OnlineMatchRequestCore {
  readonly peer_id: string;
}

function harness() {
  const request = (role: "host" | "guest"): FixtureRequest => ({
    role,
    peer_id: role,
    first_input_tick: 0,
  });
  return {
    host: newOnlineMatchModel<FakeCoordinatorState, FixtureRequest>(
      request("host"),
      fakeCoordinator("host"),
    ),
    guest: newOnlineMatchModel<FakeCoordinatorState, FixtureRequest>(
      request("guest"),
      fakeCoordinator("guest", "host_link"),
    ),
  };
}

function step(
  tick: number,
  lifecycle?: readonly RollbackWrappedEvent<LifecyclePayload>[],
): RollbackEventStepCore {
  return { tick, lifecycle_events: lifecycle ?? [] };
}

function lifecycle(
  kind: "goal" | "kickoff" | "full_time",
  home: number,
  away: number,
  team?: "home" | "away",
): RollbackWrappedEvent<LifecyclePayload> {
  const payload: LifecyclePayload =
    kind === "goal"
      ? { kind: "goal", team: team ?? "home", score: { home, away } }
      : kind === "kickoff"
        ? { kind: "kickoff" }
        : { kind: "full_time" };
  return { id: `lifecycle/${kind}/1`, tick: 0, domain: `lifecycle/${kind}`, ordinal: 1, payload };
}

function publishedPhases(effects: readonly OnlineMatchEffect[]): string[] {
  const phases: string[] = [];
  for (const effect of effects) {
    if (effect.kind === "send") {
      const message = fakeProtocolPort().decode(effect.wire);
      if (message?.kind === "match_phase") {
        phases.push(String((message.body as { readonly phase: string }).phase));
      }
    }
  }
  return phases;
}

function sent(effects: readonly OnlineMatchEffect[], kind: string): boolean {
  return effects.some(
    (effect) => effect.kind === "send" && fakeProtocolPort().decode(effect.wire)?.kind === kind,
  );
}

describe("online match model purity", () => {
  it("returns a fresh model and never mutates the one it was given", () => {
    const model = harness().host;
    const before = model.coordinator;
    const [next, effects] = command(model, ports(), { kind: "focus_lost" });
    expect(next).not.toBe(model);
    expect(model.notice).toBeUndefined();
    expect(model.abort_prompt).toBe(false);
    expect(next.notice).toBeDefined();
    expect(effects.length).toBe(0);
    expect(next.coordinator).toBe(before);
  });

  it("ignores an unknown event kind rather than failing", () => {
    const model = harness().host;
    const [next, effects] = command(model, ports(), { kind: "not_a_real_event" });
    expect(effects.length).toBe(0);
    expect(next.phase).toBe("playing");
    expect(next.error).toBeUndefined();
  });
});

describe("online match model local interruptions", () => {
  it("answers focus loss and controller loss with a notice and nothing else", () => {
    for (const testCase of [
      { kind: "focus_lost" as const, text: FOCUS_NOTICE },
      { kind: "controller_lost" as const, text: CONTROLLER_NOTICE },
    ]) {
      const model = harness().host;
      const [next, effects] = command(model, ports(), { kind: testCase.kind });
      expect(effects.length).toBe(0);
      expect(next.notice?.text).toBe(testCase.text);
      expect(next.phase).toBe("playing");
      expect(next.abort_prompt).toBe(false);
    }
  });

  it("explains a pause request before it will abort", () => {
    const model = harness().host;
    const [first, effects] = command(model, ports(), { kind: "pause_request" });
    expect(effects.length).toBe(0);
    expect(first.abort_prompt).toBe(true);
    expect(first.notice?.text).toBe(PAUSE_NOTICE);
    expect(first.phase).toBe("playing");
    expect(first.terminal).toBeUndefined();
  });

  it("aborts on a second pause request and ends the session", () => {
    const model = harness().host;
    const [armed] = command(model, ports(), { kind: "pause_request" });
    const [aborted, effects] = command(armed, ports(), { kind: "pause_request" });
    expect(sent(effects, "abort")).toBe(true);
    expect(aborted.phase).toBe("ended");
    expect(aborted.terminal?.reason).toBe("local_abort");
    expect(aborted.result).toBeUndefined();
    expect(exitRoute(aborted)).toBe("terminal");
    expect(ended(aborted)).toBe(true);
  });

  it("disarms the prompt on any other input", () => {
    const model = harness().host;
    const [armed] = command(model, ports(), { kind: "pause_request" });
    const [resumed, effects] = command(armed, ports(), { kind: "resume_request" });
    expect(effects.length).toBe(0);
    expect(resumed.abort_prompt).toBe(false);
    expect(resumed.phase).toBe("playing");
    expect(resumed.terminal).toBeUndefined();
  });

  it("decays a notice on the session clock", () => {
    const model = harness().host;
    const [noticed] = command(model, ports(), { kind: "focus_lost" });
    expect(noticed.notice?.remaining).toBe(NOTICE_SECONDS);
    const [ticked] = command(noticed, ports(), { kind: "tick", dt: 1 });
    expect(ticked.notice?.remaining).toBe(NOTICE_SECONDS - 1);
    const [expired] = command(ticked, ports(), { kind: "tick", dt: NOTICE_SECONDS });
    expect(expired.notice).toBeUndefined();
  });
});

describe("online match model confirmed publication", () => {
  it("opens the session with kickoff then playing on the first confirmed step", () => {
    const model = harness().host;
    const [next, effects] = command(model, ports(), { kind: "confirmed", steps: [step(0)] });
    expect(publishedPhases(effects).join(",")).toBe("kickoff,playing");
    expect(next.phase).toBe("playing");
  });

  it("publishes a goal stoppage only after the confirmed score moved", () => {
    let model = harness().host;
    [model] = command(model, ports(), { kind: "confirmed", steps: [step(0)] });
    const [scored, effects] = command(model, ports(), {
      kind: "confirmed",
      steps: [step(1, [lifecycle("goal", 1, 0, "home"), lifecycle("kickoff", 1, 0)])],
    });
    expect(publishedPhases(effects).join(",")).toBe("goal_stoppage,kickoff,playing");
    expect(scored.score.home).toBe(1);
    expect(scored.score.away).toBe(0);
  });

  it("walks the legal phase chain rather than publishing an illegal transition", () => {
    let model = harness().host;
    [model] = command(model, ports(), { kind: "confirmed", steps: [step(0)] });
    [model] = command(model, ports(), {
      kind: "confirmed",
      steps: [step(1, [lifecycle("goal", 1, 0, "home")])],
    });
    expect(model.published).toBe("playing");
    expect(model.error).toBeUndefined();
  });

  it("never lets a guest publish a match phase", () => {
    const model = harness().guest;
    const [next, effects] = command(model, ports(), {
      kind: "confirmed",
      steps: [step(0, [lifecycle("goal", 1, 0, "home")])],
    });
    expect(publishedPhases(effects).length).toBe(0);
    expect(next.published).toBeUndefined();
    expect(next.score.home).toBe(0);
  });

  it("reports a boundary checkpoint as a hash report", () => {
    const model = harness().host;
    const [, effects] = command(model, ports(), {
      kind: "checkpoints",
      checkpoints: [{ tick: 30, hash: "0123456789abcdef" }],
    });
    expect(sent(effects, "hash_report")).toBe(true);
  });

  it("surfaces a peer's boundary hash to the driver as well as the session", () => {
    const model = harness().host;
    const wire = fakeProtocolPort().encode({
      kind: "hash_report",
      body: { tick: 30, boundary_hash: "0123456789abcdef" },
    });
    const [, effects] = command(model, ports(), { kind: "control", link_id: "guest_1", wire });
    const observed = effects.find((effect) => effect.kind === "observe_hash");
    expect(observed?.kind === "observe_hash" ? observed.tick : undefined).toBe(30);
    expect(observed?.kind === "observe_hash" ? observed.hash : undefined).toBe("0123456789abcdef");
  });
});

describe("online match model full time and result", () => {
  function reachFullTime(model: OnlineMatchModel<FakeCoordinatorState, FixtureRequest>) {
    let next = model;
    [next] = command(next, ports(), { kind: "confirmed", steps: [step(0)] });
    return command(next, ports(), {
      kind: "full_time",
      final_tick: 90,
      home_score: 2,
      away_score: 1,
      final_hash: "0123456789abcdef",
    });
  }

  it("walks to full time and reports the simulation's own result", () => {
    const [finished, effects] = reachFullTime(harness().host);
    expect(publishedPhases(effects).join(",")).toBe("full_time");
    expect(sent(effects, "result_ack")).toBe(true);
    expect(finished.phase).toBe("full_time");
    expect(finished.score.home).toBe(2);
    expect(finished.score.away).toBe(1);
  });

  it("does not commit a result while any peer has not acknowledged it", () => {
    const [finished] = reachFullTime(harness().host);
    expect(finished.result).toBeUndefined();
    expect(finished.phase).toBe("full_time");
    expect(ended(finished)).toBe(false);
  });

  it("makes a guest report its own result rather than accept one on faith", () => {
    const [finished, effects] = reachFullTime(harness().guest);
    expect(publishedPhases(effects).length).toBe(0);
    expect(finished.phase).toBe("full_time");
    const localResult = finished.coordinator.local_result;
    expect(localResult?.home_score).toBe(2);
    expect(localResult?.away_score).toBe(1);
    expect(localResult?.final_hash).toBe("0123456789abcdef");
    expect(finished.result).toBeUndefined();
  });

  it("ignores a second full time", () => {
    const [first] = reachFullTime(harness().host);
    const [second, effects] = command(first, ports(), {
      kind: "full_time",
      final_tick: 91,
      home_score: 9,
      away_score: 9,
      final_hash: "ffffffffffffffff",
    });
    expect(effects.length).toBe(0);
    expect(second.score.home).toBe(2);
    expect(second.score.away).toBe(1);
  });
});

describe("online match model terminal reporting", () => {
  it("reports a netcode failure class into the session", () => {
    for (const testCase of [
      { failure: "late_input", reason: "late_input" },
      { failure: "desync", reason: "hash_mismatch" },
      { failure: "input_channel", reason: "input_channel_failure" },
    ]) {
      const model = harness().host;
      const [next] = command(model, ports(), {
        kind: "failure",
        failure: testCase.failure,
        detail: "a driver terminal",
      });
      expect(next.terminal?.reason).toBe(testCase.reason);
      expect(next.phase).toBe("ended");
      expect(next.result).toBeUndefined();
      expect(exitRoute(next)).toBe("terminal");
    }
  });

  it("reports a lost star as the guest's own lost host link", () => {
    const model = harness().guest;
    const [next, effects] = command(model, ports(), {
      kind: "failure",
      detail: "the star transport is no longer connected",
    });
    expect(next.terminal?.reason).toBe("transport_lost");
    expect(next.phase).toBe("ended");
    expect(publishedPhases(effects).length).toBe(0);
  });

  it("ends a host session explicitly when it cannot name the lost peer", () => {
    const model = harness().host;
    const [next, effects] = command(model, ports(), {
      kind: "failure",
      detail: "the star transport is no longer connected",
    });
    // The driver folds every link event into one status, so a host has no
    // peer id to disconnect. It ends the session rather than guessing.
    expect(next.terminal?.reason).toBe("local_abort");
    expect(sent(effects, "abort")).toBe(true);
    expect(next.phase).toBe("ended");
  });

  it("keeps the first terminal reason when late traffic follows it", () => {
    const model = harness().host;
    const [ended_] = command(model, ports(), { kind: "failure", failure: "late_input" });
    const [after, effects] = command(ended_, ports(), { kind: "confirmed", steps: [step(0)] });
    expect(effects.length).toBe(0);
    expect(after.terminal?.reason).toBe("late_input");
  });
});
