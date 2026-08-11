// The TypeScript lint gate (#471).
//
// Until this file existed there was no lint or formatting gate for TypeScript
// at all -- not a weak one, none. `stylua --check` and `lua-language-server
// --check` used to be the formatting and lint gates; they applied to Lua, and
// #467 deleted Lua. TypeScript is now roughly half the codebase and
// `scripts/check.sh` was type-checking it and nothing more.
//
// `tsc` is genuinely strict here (`noUncheckedIndexedAccess`,
// `exactOptionalPropertyTypes`), but a type-checker cannot catch a floating
// promise, a dead import, or an `any` that silently switches type-checking off
// for everything downstream of it. In packages/render/src/rig3d/** in
// particular, an unawaited promise is the shape of defect that reaches a
// frame.
//
// TYPE-AWARE ON PURPOSE. `no-floating-promises` is not a syntactic rule: it
// asks a real TypeScript program whether an expression is Promise-like. That
// is why `projectService` is wired below, and why the parser reads a
// TypeScript 6 compiler API out of tools/lint/ rather than the root's pinned
// typescript@7 -- see tools/lint/tseslint.mjs for the whole story. A run that
// silently lost its type information would not error; it would just stop
// finding floating promises. scripts/check.sh's `gate_ts_lint` guards against
// that from the outside by requiring the rule to still be REPORTED as enabled
// and a floor on the number of files linted.
import js from "@eslint/js";

import tseslint from "./tools/lint/tseslint.mjs";

export default tseslint.config(
  {
    // Build output, dependencies, and the wasm-bindgen-GENERATED glue under
    // packages/wasm/dist -- machine-written, gitignored, and not ours to
    // format or lint.
    ignores: ["**/node_modules/**", "**/dist/**", "**/dist-app/**", "**/*.tsbuildinfo"],
  },

  {
    // A stale `eslint-disable` is a claim about the code that is no longer
    // true, and the tree already carried six of them before this gate existed.
    // ESLint reports them as WARNINGS by default; `scripts/check.sh` runs with
    // --max-warnings 0, and this makes them errors under a bare `eslint .` too.
    linterOptions: { reportUnusedDisableDirectives: "error" },
  },

  js.configs.recommended,
  tseslint.configs.recommended,
  tseslint.configs.recommendedTypeChecked,

  {
    languageOptions: {
      parserOptions: {
        // The project service, not a static `project` list: it follows each
        // file to the tsconfig that actually owns it, which is what makes the
        // ten package projects work without restating them here.
        projectService: {
          // The six files no package's `rootDir: "src"` covers. They are
          // routed to tsconfig.eslint.json (whose `include` is empty, as the
          // project service requires). Adding a seventh tooling file to the
          // tree fails LOUDLY here rather than silently dropping its type
          // information.
          allowDefaultProject: [
            "vite.config.ts",
            "vitest.config.ts",
            "eslint.config.mjs",
            "tools/lint/tseslint.mjs",
            "packages/wasm/scripts/build.mjs",
            "packages/wasm/scripts/build_web.mjs",
          ],
          defaultProject: "tsconfig.eslint.json",
        },
        tsconfigRootDir: import.meta.dirname,
      },
    },
  },

  {
    rules: {
      // ---- The three #471 names explicitly -------------------------------
      //
      // 1. no-floating-promises. The reason this gate is type-aware. An
      //    unawaited promise in the renderer or the online session does not
      //    throw where it was written; it rejects into nothing, and the
      //    symptom arrives later and somewhere else. `ignoreVoid` keeps the
      //    deliberate, documented `void somethingAsync()` spelling available
      //    as the escape hatch, so suppressing one never needs a disable
      //    comment.
      "@typescript-eslint/no-floating-promises": ["error", { ignoreVoid: true, ignoreIIFE: false }],

      // 2. no-unused-vars. `tsconfig.base.json` sets neither
      //    `noUnusedLocals` nor `noUnusedParameters`, so nothing caught a dead
      //    import or an abandoned local before this. A leading underscore is
      //    the opt-out, which is what makes deliberately-ignored positional
      //    parameters expressible.
      "@typescript-eslint/no-unused-vars": [
        "error",
        {
          args: "after-used",
          argsIgnorePattern: "^_",
          varsIgnorePattern: "^_",
          caughtErrorsIgnorePattern: "^_",
          destructuredArrayIgnorePattern: "^_",
          ignoreRestSiblings: true,
        },
      ],

      // 3. no-explicit-any. ARCHITECTURE.md §4 rule 1 already forbids `any`,
      //    in prose, enforced by nothing. This is the enforcement. Left at
      //    `error` with no `fixToUnknown`: the correct replacement is almost
      //    never a blind `unknown`, it is the type the author knew and did not
      //    write down.
      "@typescript-eslint/no-explicit-any": "error",

      // ---- Everything else is chosen, not inherited ----------------------

      // `no-undef` is TypeScript's job and it does it better: on TS sources
      // this rule only produces false positives for globals the compiler
      // already knows about. The typescript-eslint docs say to turn it off,
      // and `tsc --build --force` in scripts/check.sh is the real check.
      "no-undef": "off",

      // Prefer the type-aware version of each of these; the base rule cannot
      // see through types and double-reports.
      "no-unused-vars": "off",

      // Not inherited from any preset -- enabled because the tree was already
      // carrying `// eslint-disable-next-line no-console` comments written
      // against a linter that did not exist yet, i.e. somebody already
      // considered a bare `console.log` in shipped game code to be debug
      // residue. `warn`/`error` stay allowed: those are diagnostics a player's
      // console is supposed to receive.
      "no-console": ["error", { allow: ["warn", "error"] }],

      // `ignoreStatic` because a `static` method reference carries no
      // instance `this` to lose -- `{ new: CompatibilityMetrics.new }` and
      // `options.eval ?? BrowserTransport._defaultEval` are both correct, and
      // reporting them would be teaching the reader to ignore the rule.
      "@typescript-eslint/unbound-method": ["error", { ignoreStatic: true }],

      // Same reasoning: `browser_star_bridge.ts` already carried a
      // `// eslint-disable-next-line no-eval`. That one use is deliberate and
      // documented (indirect eval, global scope, for the browser test bridge);
      // enabling the rule is what keeps the NEXT one from being silent.
      "no-eval": "error",
    },
  },

  {
    // The wasm build scripts and this tree's own tooling. `console.log` is
    // their output -- suppressing it would be suppressing the tool.
    files: [
      "packages/wasm/scripts/*.mjs",
      "vite.config.ts",
      "vitest.config.ts",
      "eslint.config.mjs",
      "tools/lint/*.mjs",
    ],
    rules: { "no-console": "off" },
  },

  {
    // Vitest specs and the render benchmark. Rules relax here for one reason:
    // a test deliberately constructs shapes the production types forbid, in
    // order to assert what happens to them, and requiring the full production
    // type would mean restating the whole shape in every test.
    files: ["**/*.spec.ts", "**/*.test.ts", "packages/render/src/benchmark.ts"],
    rules: {
      "@typescript-eslint/no-unsafe-assignment": "off",
      "@typescript-eslint/no-unsafe-member-access": "off",
      "@typescript-eslint/no-unsafe-argument": "off",
      "@typescript-eslint/no-unsafe-call": "off",
      "@typescript-eslint/no-unsafe-return": "off",
      // `expect(obj.method)` is the standard vitest spelling and is not the
      // `this`-losing mistake the rule is about. The benchmark prints its
      // results; that is its whole job.
      "@typescript-eslint/unbound-method": "off",
      "no-console": "off",
    },
  },
);
