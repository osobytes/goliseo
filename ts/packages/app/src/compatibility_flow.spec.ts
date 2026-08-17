// The second case ("drives the production bootstrap into and out of the
// real match") exercises `bootstrap.new` wired through `real_match_factory.ts`
// against `@gc/screens`'s `real_match.ts`/`match.ts`. It flips the
// underlying fake `SimHostPort`'s `hud.finished` rather than a `finished`
// field on the screen directly (the same seam `bootstrap.spec.ts`'s "is the
// adapter selected by the default bootstrap" test uses, for the identical
// reason -- `RealMatchScreenPort.state` has no settable `finished` field,
// only a `time_left`/`score` getter derived from the host's HUD).

import { describe, expect, it } from "vitest";
import { App } from "./app.ts";
import { bootstrap } from "./bootstrap.ts";
import { CompatibilityFlow } from "./compatibility_flow.ts";
import { createRealMatchFactory } from "./real_match_factory.ts";
import {
  APP_CONTENT,
  fakeHostFactory,
  fakeKeyboard,
  MATCH_CONTRACT_CONTENT,
  noopRenderPort,
} from "./test_support/fixtures.ts";

describe("compatibility flow", () => {
  it("drives the complete fake product flow through the normal input seam", () => {
    const inputs: string[] = [];
    const flow = new CompatibilityFlow((kind) => {
      inputs.push(kind);
    });
    flow.actionDelay = 0;
    const app = new App(APP_CONTENT);

    for (let step = 1; step <= 8; step += 1) {
      flow.update(app, step);
    }

    expect(app.currentRoute()).toBe("result");
    expect(flow.finished).toBe(true);
    // Two fewer clicks than before: the pre-match trio is one screen.
    const expected = ["compat_click_play", "compat_click_kickoff", "compat_click_complete"];
    expect(inputs).toEqual(expected);
  });

  it("drives the production bootstrap into and out of the real match", () => {
    const flow = new CompatibilityFlow();
    flow.actionDelay = 0;
    const { createHost, hosts } = fakeHostFactory();
    const factory = createRealMatchFactory({
      content: MATCH_CONTRACT_CONTENT,
      createHost,
      renderer: noopRenderPort,
      keyboard: fakeKeyboard(),
    });
    const app = bootstrap.new(APP_CONTENT, factory, 960, 540, {
      settingsStorage: { read: () => undefined, write: () => ({ ok: true, value: true }) },
    });

    for (let step = 1; step <= 4; step += 1) {
      flow.update(app, step);
    }

    expect(app.adapter.kind).toBe("real");
    expect(app.currentRoute()).toBe("match");

    app.focus(false);
    expect(app.currentRoute()).toBe("pause");

    flow.update(app, 4.5);
    expect(app.currentRoute()).toBe("pause");
    flow.update(app, 5.0);
    expect(app.currentRoute()).toBe("match");

    const host = hosts[hosts.length - 1];
    if (!host) {
      throw new Error("expected a fake sim host to have been constructed");
    }
    host.hud.finished = true;
    app.update(0.9); // >= real_match.ts's FULL_TIME_HOLD (0.9s)

    flow.update(app, 5);
    expect(app.currentRoute()).toBe("result");
    expect(flow.finished).toBe(true);
  });
});
