// The lint gate for infra/, self-contained from the ts/ game workspace's
// own eslint.config.mjs (which pins typescript-eslint to a typescript@6
// shim -- see ts/tools/lint/tseslint.mjs -- to work around ts/'s
// typescript@7 having no JS compiler API at all). infra/ pins a plain
// typescript@5.9 (package.json), so typescript-eslint's type-aware rules
// work directly against it with no such seam needed.
import js from "@eslint/js";
import tseslint from "typescript-eslint";

export default tseslint.config(
  {
    // Tooling config, not application code: not worth a dedicated
    // tsconfig.eslint.json (ts/'s own pattern) for two small files.
    ignores: [
      "**/node_modules/**",
      "**/dist/**",
      "**/.wrangler/**",
      "worker-configuration.d.ts",
      "eslint.config.mjs",
      "vitest.config.ts",
    ],
  },

  { linterOptions: { reportUnusedDisableDirectives: "error" } },

  js.configs.recommended,
  tseslint.configs.recommended,
  tseslint.configs.recommendedTypeChecked,

  {
    languageOptions: {
      parserOptions: {
        projectService: true,
        tsconfigRootDir: import.meta.dirname,
      },
    },
  },

  {
    rules: {
      // AGENTS.md §5: no `any`, ever.
      "@typescript-eslint/no-explicit-any": "error",
      // The point of the type-aware config: every Promise must be awaited,
      // returned, voided, or handed to ctx.waitUntil() (workers-best-practices).
      "@typescript-eslint/no-floating-promises": "error",
    },
  },
);
