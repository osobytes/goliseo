// Secrets are deliberately absent from wrangler.jsonc (never committed --
// see docs/online/hosting_runbook.md for how the owner sets them with
// `wrangler secret put`), so `wrangler types` cannot discover them from
// config the way it does bindings and vars. This ambient declaration
// merges into the generated `Env` interface in worker-configuration.d.ts
// (TypeScript interface merging across files, both in this package's
// `include`) to add exactly the two secret-shaped fields index.ts reads.
//
// Both are optional: an unset secret is not a type error, it is the
// documented "TURN not configured yet" path -- handleTurnCredentials in
// index.ts checks for it explicitly and returns 404.
interface Env {
  readonly TURN_KEY_ID?: string;
  readonly TURN_API_TOKEN?: string;
}
