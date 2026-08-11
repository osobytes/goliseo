// Ported from game/match_onboarding.lua.

export type OnboardingPromptId = "move" | "equipment" | "possession" | "defending" | "keeper";

export interface OnboardingPrompt {
  readonly id: OnboardingPromptId;
  readonly title: string;
  readonly body: string;
}

export interface OnboardingContext {
  readonly carrying: boolean;
  readonly defending: boolean;
  readonly keeper_holding: boolean;
  readonly moved: boolean;
  readonly shot: boolean;
  readonly passed: boolean;
  readonly defended: boolean;
  readonly equipment_available: boolean;
  readonly equipment_used: boolean;
}

export interface MatchOnboardingState {
  readonly enabled: boolean;
  readonly current?: OnboardingPromptId;
  readonly remaining: number;
  readonly shown: Readonly<Record<string, boolean>>;
  readonly combatEnabled: boolean;
}

const PROMPT_SECONDS = 6;

const PROMPTS: Readonly<Record<OnboardingPromptId, OnboardingPrompt>> = {
  move: {
    id: "move",
    title: "YOU CONTROL THE DOUBLE-RINGED PLAYER",
    body: "MOVE [LS / WASD]     SPRINT [LB / SHIFT]",
  },
  equipment: {
    id: "equipment",
    title: "COMBAT PROTOTYPE · EQUIPMENT",
    body: "FACE YOUR TARGET     HOLD / TAP [B / J] TO USE EQUIPMENT",
  },
  possession: {
    id: "possession",
    title: "WITH THE BALL",
    body: "HOLD [A / SPACE] TO SHOOT     [X / K] TO PASS",
  },
  defending: {
    id: "defending",
    title: "WIN IT BACK",
    body: "HOLD [A / SPACE] TO JOCKEY     [X / K] SWITCHES",
  },
  keeper: {
    id: "keeper",
    title: "KEEPER IN POSSESSION",
    body: "[X / K] THROW     [A / SPACE] PUNT",
  },
};

function taughtActionUsed(current: OnboardingPromptId, context: OnboardingContext): boolean {
  if (current === "move") {
    return context.moved;
  } else if (current === "equipment") {
    return context.equipment_used;
  } else if (current === "possession") {
    return context.shot || context.passed;
  } else if (current === "defending") {
    return context.defended;
  } else if (current === "keeper") {
    return context.shot || context.passed;
  }
  return false;
}

function newState(enabled: boolean, combatEnabled?: boolean): MatchOnboardingState {
  const combat = combatEnabled === true;
  const active = enabled || combat;
  const first: OnboardingPromptId | undefined = enabled ? "move" : combat ? "equipment" : undefined;
  return {
    enabled: active,
    ...(first !== undefined ? { current: first } : {}),
    remaining: active ? PROMPT_SECONDS : 0,
    shown: first !== undefined ? { [first]: true } : {},
    combatEnabled: combat,
  };
}

function update(state: MatchOnboardingState, context: OnboardingContext, dt: number): MatchOnboardingState {
  if (!state.enabled) {
    return state;
  }

  let current = state.current;
  let remaining = state.remaining;
  const shown: Record<string, boolean> = { ...state.shown };
  if (current !== undefined) {
    remaining = Math.max(0, remaining - dt);
    if (remaining === 0 || taughtActionUsed(current, context)) {
      current = undefined;
    }
  }

  let candidate: OnboardingPromptId | undefined;
  if (current === undefined) {
    if (state.combatEnabled && context.equipment_available && shown.equipment !== true) {
      candidate = "equipment";
    } else if (context.keeper_holding && shown.keeper !== true) {
      candidate = "keeper";
    } else if (context.carrying && shown.possession !== true) {
      candidate = "possession";
    } else if (context.defending && shown.defending !== true) {
      candidate = "defending";
    }
  }
  if (candidate !== undefined) {
    current = candidate;
    remaining = PROMPT_SECONDS;
    shown[candidate] = true;
  }

  return {
    enabled: state.enabled,
    ...(current !== undefined ? { current } : {}),
    remaining,
    shown,
    combatEnabled: state.combatEnabled,
  };
}

function prompt(state: MatchOnboardingState): OnboardingPrompt | undefined {
  return state.current !== undefined ? PROMPTS[state.current] : undefined;
}

export const onboarding = { new: newState, update, prompt };
