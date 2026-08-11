import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    include: ["packages/*/src/**/*.spec.ts"],
    environment: "node",

    // Vitest's 5s default is right for the ~800 unit tests here and wrong for
    // the handful that drive a real wasm-compiled match end to end -- a
    // 7,200-tick simulation, or a rollback lab converging over hundreds of
    // resimulated ticks. Those take 1-2s alone and 5-9s when the full suite
    // is competing for cores, so the suite failed only when run whole, which
    // is the worst way for a gate to be wrong: green per package, red in CI.
    //
    // This is NOT licence for slow unit tests. Anything here that is not
    // driving wasm should still finish in milliseconds; if a pure-logic spec
    // starts needing seconds, that is a defect in the spec, not a reason to
    // raise this further.
    testTimeout: 30_000,
  },
});
