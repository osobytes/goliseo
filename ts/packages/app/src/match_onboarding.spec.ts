import { describe, expect, it } from "vitest";
import { onboarding, type OnboardingContext } from "./match_onboarding.ts";

function context(values: Partial<OnboardingContext> = {}): OnboardingContext {
  return {
    carrying: values.carrying === true,
    defending: values.defending === true,
    keeper_holding: values.keeper_holding === true,
    moved: values.moved === true,
    shot: values.shot === true,
    passed: values.passed === true,
    defended: values.defended === true,
    equipment_available: values.equipment_available === true,
    equipment_used: values.equipment_used === true,
  };
}

describe("first-match onboarding", () => {
  it("teaches one contextual action at a time and never repeats a lesson", () => {
    let state = onboarding.new(true);
    expect(onboarding.prompt(state)?.id).toBe("move");

    state = onboarding.update(state, context({ moved: true, carrying: true }), 0.1);
    expect(onboarding.prompt(state)?.id).toBe("possession");
    state = onboarding.update(state, context({ passed: true, carrying: true }), 0.1);
    expect(onboarding.prompt(state)).toBeUndefined();

    state = onboarding.update(state, context({ defending: true }), 0.1);
    expect(onboarding.prompt(state)?.id).toBe("defending");
    state = onboarding.update(state, context({ defending: true, defended: true }), 0.1);
    expect(onboarding.prompt(state)).toBeUndefined();
    state = onboarding.update(state, context({ defending: true }), 0.1);
    expect(onboarding.prompt(state)).toBeUndefined();
  });

  it("prioritizes keeper distribution and retires prompts after six seconds", () => {
    let state = onboarding.new(true);
    state = onboarding.update(state, context(), 6);
    expect(onboarding.prompt(state)).toBeUndefined();
    state = onboarding.update(state, context({ keeper_holding: true, carrying: true }), 0);
    expect(onboarding.prompt(state)?.id).toBe("keeper");
    state = onboarding.update(state, context({ keeper_holding: true, carrying: true, shot: true }), 0.1);
    expect(onboarding.prompt(state)?.id).toBe("possession");
  });

  it("is entirely suppressed after the first fixture", () => {
    let state = onboarding.new(false);
    state = onboarding.update(state, context({ moved: true, carrying: true, passed: true }), 10);
    expect(onboarding.prompt(state)).toBeUndefined();
  });

  it("teaches equipment only in the explicit combat prototype", () => {
    let combat = onboarding.new(false, true);
    expect(onboarding.prompt(combat)?.id).toBe("equipment");
    combat = onboarding.update(combat, context({ equipment_available: true, equipment_used: true }), 0.1);
    expect(onboarding.prompt(combat)).toBeUndefined();

    let showcase = onboarding.new(false, false);
    showcase = onboarding.update(showcase, context({ equipment_available: true }), 0.1);
    expect(onboarding.prompt(showcase)).toBeUndefined();
  });
});
