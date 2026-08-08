// Minimal ambient typing for the one Vite-injected global `browser_main.ts`
// reads (`import.meta.env.DEV`), instead of pulling in the full
// `vite/client` type package (which also declares `import.meta.glob`, CSS
// module imports, asset-URL imports, and more this app shell does not use)
// as a new dependency of `@gc/app` just for one boolean.
interface ImportMetaEnv {
  readonly DEV: boolean;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
