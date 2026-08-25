// Minimal ambient typing for the Vite-injected globals this app shell
// reads, instead of pulling in the full `vite/client` type package (which
// also declares `import.meta.glob`, CSS module imports, asset-URL imports,
// and more this app shell does not use) as a new dependency of `@gc/app`.
interface ImportMetaEnv {
  readonly DEV: boolean;
  /**
   * The deployed commit's short SHA, set by `.github/workflows/deploy.yml`
   * as a `VITE_`-prefixed env var before `vite build` -- Vite exposes any
   * such variable on `import.meta.env` with no further config. `undefined`
   * for every build that does not set it (local `vite dev`/`vite build`,
   * `vitest`, `ci.yml`'s gate build); `build_info.ts`'s `buildInfo.build_sha`
   * is where the "dev" fallback for that case lives (#612).
   */
  readonly VITE_BUILD_SHA?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
