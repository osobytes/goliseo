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
// is the native `tsgo` port -- a from-scratch reimplementation in Go, not a
// newer release of the JavaScript compiler. The `tsc` it installs is a
// statically linked Go binary. Its npm package therefore deliberately ships no
// JS compiler API: its whole `exports["."]` is `lib/version.cjs`, which exports
// exactly two things:
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
// THE LIMIT OF THIS ARRANGEMENT, and it is the most important caveat about the
// whole gate, so do not over-read a green run.
//
// The linter and the compiler are two different implementations, so they can
// disagree. `tsc --build --force` catches such a disagreement only when it
// also produces a real TYPE ERROR -- which is exactly what happened at
// packages/transport/src/fake_relay.ts, where typescript@6 called an assertion
// unnecessary and typescript@7 then failed with TS18048. That is a good
// backstop and it fired for real.
//
// It cannot catch a disagreement about whether an expression is Promise-like,
// because `tsc` has no opinion on floating promises at all -- that is the
// entire reason this gate exists. If the two compilers ever diverged there,
// the lint would silently under-report and nothing downstream would notice.
// This is still strictly better than the zero detection that preceded #471;
// it is simply not a proof.
//
// WHEN TO DELETE THIS. The moment typescript-eslint can read type information
// from TypeScript 7, delete this package, drop `typescript` from it, and
// import `typescript-eslint` directly in eslint.config.mjs. Nothing else in
// the tree depends on this seam.
//
// Upstream tracking issue, checked 2026-08-11 and open, labelled "blocked by
// external API":
//   https://github.com/typescript-eslint/typescript-eslint/issues/10940
//   ("Enhancement: Use TS 7 (tsgo / typescript-go) for type information")
// The incompatibility itself is confirmed in issue 12518 ("TypeScript 7.0.2
// Support", closed as not planned), which records the same peer range quoted
// above and the "Cannot read properties of undefined" crash you get by forcing
// the install anyway.
//
// scripts/check.sh does not take that on faith either: `check_tseslint_peer()`
// reads the installed package's declared peer range on every gate run and
// fails the moment it stops being the one this workaround was written against,
// so nobody has to remember to come back and check.
export { default } from "typescript-eslint";
