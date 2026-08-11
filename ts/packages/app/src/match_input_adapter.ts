// Offline input sampling is render-rate driven, but legacy MatchInput
// records are consumed only by the fixed simulation clock. Pending edges
// survive a zero-tick render update and are emitted once on the next
// simulated tick.

import { Vec2 } from "@gc/core";

export interface MatchInputEdges {
  readonly shoot: boolean;
  readonly pass: boolean;
  readonly switch: boolean;
  readonly dash: boolean;
  readonly dodge: boolean;
  /** Captures the modifier paired with a pending release edge. */
  readonly lob: boolean;
}

export interface MatchInputHeld {
  readonly move: Vec2;
  readonly shoot_held: boolean;
  readonly pass_held: boolean;
  readonly lob: boolean;
  readonly sprint: boolean;
  readonly jockey: boolean;
  readonly aerial_strike: boolean;
  readonly aerial_acrobatic: boolean;
  readonly equipment_held: boolean;
}

export interface MatchInputAdapterState {
  readonly held: MatchInputHeld;
  readonly pending: MatchInputEdges;
  readonly equipment_sampled_held: boolean;
  readonly equipment_transitioned: boolean;
}

/** The one legacy `MatchInput` record produced per simulated tick (`gc-sim`'s `match.rs` input shape). */
export interface MatchInput {
  readonly move: Vec2;
  readonly shoot: boolean;
  readonly shoot_held: boolean;
  readonly pass: boolean;
  readonly pass_held: boolean;
  readonly switch: boolean;
  readonly dash: boolean;
  readonly dodge: boolean;
  readonly lob: boolean;
  readonly sprint: boolean;
  readonly jockey: boolean;
  readonly aerial_strike?: boolean;
  readonly aerial_acrobatic?: boolean;
  readonly equipment_held: boolean;
  readonly equipment_pressed: boolean;
  readonly equipment_released: boolean;
}

function noEdges(): MatchInputEdges {
  return { shoot: false, pass: false, switch: false, dash: false, dodge: false, lob: false };
}

function neutralHeld(): MatchInputHeld {
  return {
    move: new Vec2(0, 0),
    shoot_held: false,
    pass_held: false,
    lob: false,
    sprint: false,
    jockey: false,
    aerial_strike: false,
    aerial_acrobatic: false,
    equipment_held: false,
  };
}

function newState(): MatchInputAdapterState {
  return {
    held: neutralHeld(),
    pending: noEdges(),
    equipment_sampled_held: false,
    equipment_transitioned: false,
  };
}

// Capture one render-sampled input without consuming its one-shot actions.
function sample(state: MatchInputAdapterState, input: MatchInput): MatchInputAdapterState {
  const equipmentTransitioned =
    state.equipment_transitioned ||
    input.equipment_pressed ||
    input.equipment_released ||
    state.held.equipment_held !== input.equipment_held;
  return {
    held: {
      move: input.move,
      shoot_held: input.shoot_held,
      pass_held: input.pass_held,
      lob: input.lob,
      sprint: input.sprint,
      jockey: input.jockey,
      aerial_strike: input.aerial_strike === true,
      aerial_acrobatic: input.aerial_acrobatic === true,
      equipment_held: input.equipment_held,
    },
    pending: {
      shoot: state.pending.shoot || input.shoot,
      pass: state.pending.pass || input.pass,
      switch: state.pending.switch || input.switch,
      dash: state.pending.dash || input.dash,
      dodge: state.pending.dodge || input.dodge,
      lob: state.pending.lob || (input.lob && (input.shoot || input.pass)),
    },
    equipment_sampled_held: state.equipment_sampled_held,
    equipment_transitioned: equipmentTransitioned,
  };
}

// Produce one legacy MatchInput for a single clock tick, then clear only the
// edges that have been consumed. Holds remain live for every catch-up tick.
function nextTick(state: MatchInputAdapterState): readonly [MatchInputAdapterState, MatchInput] {
  const held = state.held;
  const pending = state.pending;
  let equipmentPressed = false;
  let equipmentReleased = false;
  if (state.equipment_transitioned) {
    if (state.equipment_sampled_held) {
      if (held.equipment_held) {
        equipmentPressed = true;
      } else {
        equipmentReleased = true;
      }
    } else {
      equipmentPressed = true;
      equipmentReleased = !held.equipment_held;
    }
  }
  return [
    {
      held,
      pending: noEdges(),
      equipment_sampled_held: held.equipment_held,
      equipment_transitioned: false,
    },
    {
      move: held.move,
      shoot: pending.shoot,
      shoot_held: held.shoot_held,
      pass: pending.pass,
      pass_held: held.pass_held,
      switch: pending.switch,
      dash: pending.dash,
      dodge: pending.dodge,
      lob: held.lob || pending.lob,
      sprint: held.sprint,
      jockey: held.jockey,
      aerial_strike: held.aerial_strike,
      aerial_acrobatic: held.aerial_acrobatic,
      equipment_held: held.equipment_held,
      equipment_pressed: equipmentPressed,
      equipment_released: equipmentReleased,
    },
  ];
}

export const matchInputAdapter = { new: newState, sample, nextTick };
