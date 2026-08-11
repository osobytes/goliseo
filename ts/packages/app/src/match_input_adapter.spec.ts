// Ported from spec/game/match_input_adapter_spec.lua.
//
// `sim/fixed_clock.lua` is Rust-owned (`sim/**` -> `crates/gc-sim`;
// v2/README.md §2), and this package must not depend on Rust sources (rule
// 6.7). The Lua spec drives `match_input_adapter` through
// `fixed_clock.advance`, which is pure render/tick-cadence bookkeeping with
// no simulation math in it -- unlike the transcendental-call determinism
// concerns the README warns about. Reimplementing just its `new`/`advance`
// shape as a local, test-only double (not exported, not used by production
// code) is the smallest way to keep this spec's assertions about
// `match_input_adapter`'s own edge-holding behavior faithful, without
// duplicating anything simulation-critical into a shipped module. A real
// `FixedClockPort` (mirroring `RollbackEventsPort` et al.) is the fix once a
// wasm-compiled `gc-sim` exists; see rollback_validation.ts's ports for the
// established pattern.

import { describe, expect, it } from "vitest";
import { Vec2 } from "@gc/core";
import { matchInputAdapter, type MatchInput } from "./match_input_adapter.ts";

const TICK_SECONDS = 1 / 60;
const MAX_TICKS_PER_UPDATE = 8;
const EPSILON = TICK_SECONDS * 1e-9;

interface FixedClockState {
  tick: number;
  accumulator: number;
}

function newClock(): FixedClockState {
  return { tick: 0, accumulator: 0 };
}

function advance(
  state: FixedClockState,
  renderDt: number,
  inputForTick: (tick: number) => MatchInput,
  step: (tick: number, input: MatchInput) => void,
): void {
  state.accumulator += renderDt;
  let ticks = 0;
  while (state.accumulator + EPSILON >= TICK_SECONDS && ticks < MAX_TICKS_PER_UPDATE) {
    const tick = state.tick;
    const input = inputForTick(tick);
    step(tick, input);
    state.tick = tick + 1;
    state.accumulator -= TICK_SECONDS;
    if (state.accumulator < 0) {
      state.accumulator = 0;
    }
    ticks += 1;
  }
}

interface InputOptions {
  shoot?: boolean;
  shoot_held?: boolean;
  pass?: boolean;
  pass_held?: boolean;
  switch?: boolean;
  dash?: boolean;
  dodge?: boolean;
  lob?: boolean;
  sprint?: boolean;
  jockey?: boolean;
  equipment_held?: boolean;
  equipment_pressed?: boolean;
  equipment_released?: boolean;
}

function input(opts: InputOptions = {}): MatchInput {
  return {
    move: new Vec2(0.5, -0.25),
    shoot: opts.shoot === true,
    shoot_held: opts.shoot_held === true,
    pass: opts.pass === true,
    pass_held: opts.pass_held === true,
    switch: opts.switch === true,
    dash: opts.dash === true,
    dodge: opts.dodge === true,
    lob: opts.lob === true,
    sprint: opts.sprint === true,
    jockey: opts.jockey === true,
    equipment_held: opts.equipment_held === true,
    equipment_pressed: opts.equipment_pressed === true,
    equipment_released: opts.equipment_released === true,
  };
}

describe("offline match input adapter", () => {
  it("holds a release edge through a zero-tick render update and consumes it once", () => {
    let adapter = matchInputAdapter.sample(matchInputAdapter.new(), input({ shoot: true, lob: true }));
    const clock = newClock();
    const consumed: MatchInput[] = [];

    advance(
      clock,
      1 / 120,
      () => {
        const [next, tickInput] = matchInputAdapter.nextTick(adapter);
        adapter = next;
        return tickInput;
      },
      (_tick, tickInput) => {
        consumed.push(tickInput);
      },
    );
    expect(consumed.length).toBe(0);

    advance(
      clock,
      1 / 120,
      () => {
        const [next, tickInput] = matchInputAdapter.nextTick(adapter);
        adapter = next;
        return tickInput;
      },
      (_tick, tickInput) => {
        consumed.push(tickInput);
      },
    );
    expect(consumed.length).toBe(1);
    expect(consumed[0]?.shoot).toBe(true);
    expect(consumed[0]?.lob).toBe(true);

    const [, later] = matchInputAdapter.nextTick(adapter);
    expect(later.shoot).toBe(false);
    expect(later.lob).toBe(true);

    const nextAdapter = matchInputAdapter.sample(adapter, input());
    const [, released] = matchInputAdapter.nextTick(nextAdapter);
    expect(released.lob).toBe(false);
  });

  it("emits edges only on the first catch-up tick while preserving holds", () => {
    const adapter = matchInputAdapter.sample(
      matchInputAdapter.new(),
      input({ pass: true, pass_held: true, sprint: true, jockey: true, equipment_held: true, equipment_pressed: true }),
    );
    const [firstState, first] = matchInputAdapter.nextTick(adapter);
    const [, second] = matchInputAdapter.nextTick(firstState);
    expect(first.pass).toBe(true);
    expect(first.pass_held).toBe(true);
    expect(first.sprint).toBe(true);
    expect(first.jockey).toBe(true);
    expect(first.equipment_held).toBe(true);
    expect(first.equipment_pressed).toBe(true);
    expect(first.equipment_released).toBe(false);
    expect(second.pass).toBe(false);
    expect(second.pass_held).toBe(true);
    expect(second.sprint).toBe(true);
    expect(second.jockey).toBe(true);
    expect(second.equipment_held).toBe(true);
    expect(second.equipment_pressed).toBe(false);
    expect(second.equipment_released).toBe(false);
  });

  it("folds all canonical equipment transition sequences", () => {
    function fold(previousHeld: boolean, samples: readonly MatchInput[]): MatchInput {
      let adapter = matchInputAdapter.new();
      if (previousHeld) {
        adapter = matchInputAdapter.sample(adapter, input({ equipment_held: true, equipment_pressed: true }));
        [adapter] = matchInputAdapter.nextTick(adapter);
      }
      for (const s of samples) {
        adapter = matchInputAdapter.sample(adapter, s);
      }
      const [, tickInput] = matchInputAdapter.nextTick(adapter);
      return tickInput;
    }

    const upIdle = fold(false, [input()]);
    expect(upIdle.equipment_held).toBe(false);
    expect(upIdle.equipment_pressed).toBe(false);
    expect(upIdle.equipment_released).toBe(false);

    const pressed = fold(false, [input({ equipment_held: true })]);
    expect(pressed.equipment_held).toBe(true);
    expect(pressed.equipment_pressed).toBe(true);
    expect(pressed.equipment_released).toBe(false);

    const fastTap = fold(false, [
      input({ equipment_held: true, equipment_pressed: true }),
      input({ equipment_released: true }),
    ]);
    expect(fastTap.equipment_held).toBe(false);
    expect(fastTap.equipment_pressed).toBe(true);
    expect(fastTap.equipment_released).toBe(true);

    const downIdle = fold(true, [input({ equipment_held: true })]);
    expect(downIdle.equipment_held).toBe(true);
    expect(downIdle.equipment_pressed).toBe(false);
    expect(downIdle.equipment_released).toBe(false);

    const released = fold(true, [input({ equipment_released: true })]);
    expect(released.equipment_held).toBe(false);
    expect(released.equipment_pressed).toBe(false);
    expect(released.equipment_released).toBe(true);

    const releaseRepress = fold(true, [
      input({ equipment_released: true }),
      input({ equipment_held: true, equipment_pressed: true }),
    ]);
    expect(releaseRepress.equipment_held).toBe(true);
    expect(releaseRepress.equipment_pressed).toBe(true);
    expect(releaseRepress.equipment_released).toBe(false);
  });
});
