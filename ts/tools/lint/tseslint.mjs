// The single import seam between ../../eslint.config.mjs and typescript-eslint.
//
// WHY THIS FILE EXISTS AT ALL, rather than the config importing
// `typescript-eslint` directly.
//
// typescript-eslint's type-aware rules -- `no-floating-promises` above all,
// the rule #471 was opened for -- do not pattern-match source text. They ask a
// real TypeScript program whether an expression is a Promise. To do that they
// need the TypeScript *JavaScript* compiler API: `ts.createProgram`,
// `ts.TypeChecker`, the works.
//
// This repository pins `typescript@7.0.2` (ts/package.json), and TypeScript 7
// is the native port. Its npm package deliberately ships no JS compiler API --
// its whole `exports["."]` is `lib/version.cjs`, which exports exactly two
// things:
//
//     $ node -e "import('typescript').then(m => console.log(Object.keys(m)))"
//     [ 'default', 'version', 'versionMajorMinor' ]
//
// So typescript-eslint cannot run against the compiler this repository builds
// with. Its peer range says as much: `typescript >=4.8.4 <6.1.0`.
//
// The fix is not to downgrade the build. `tsc --build --force` in
// scripts/check.sh must keep running the pinned 7.0.2 -- that is the compiler
// whose diagnostics we ship against. Instead this tiny workspace package
// carries its own `typescript@6.0.3` dependency, and pnpm's isolated
// node_modules resolves typescript-eslint's `typescript` peer to THAT copy,
// here, without the root ever seeing it. 6.0.3 is the last JS-API release and
// is the same language as 7.0 (`erasableSyntaxOnly`,
// `rewriteRelativeImportExtensions`, `exactOptionalPropertyTypes`,
// `noUncheckedIndexedAccess` all exist in it), so the program the linter reads
// is the program the compiler checks.
//
// Because Node resolves a bare specifier from the *importing file's* location,
// `eslint.config.mjs` up in ts/ cannot reach ts/tools/lint/node_modules on its
// own. It imports this file by relative path instead, and the bare specifier
// below resolves from here.
//
// WHEN TO DELETE THIS. The moment typescript-eslint supports the TypeScript 7
// API (tracked upstream as the tsgo/`@typescript/api` work), delete this
// package, drop `typescript` from it, and import `typescript-eslint` directly
// in eslint.config.mjs. Nothing else in the tree depends on this seam.
export { default } from "typescript-eslint";
