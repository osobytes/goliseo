#!/usr/bin/env node
// Generates the text of THIRD_PARTY_NOTICES.txt: the notices for every
// third-party component actually shipped in the browser bundle -- three.js
// (the one non-`@gc/*` runtime dependency of `@gc/render` / `@gc/app`) and
// every Rust crate `cargo` links into `gc_wasm_bg.wasm`.
//
// Wired into `pnpm build` by the `thirdPartyNotices` plugin in
// ts/vite.config.ts (its `closeBundle` hook calls `buildThirdPartyNotices`
// and writes the result into `dist-app/`), so this is generated on every
// build from what `pnpm` and `cargo` actually resolved -- never a
// hand-maintained list that can go stale. `THIRD_PARTY.md` in the repository
// root is the full audited inventory this is a distributed-artifact slice
// of; see its "Reproducing this audit" section for the `cargo tree` command
// this mirrors.
//
// Requires a working `cargo` on PATH (this workspace already requires one to
// build `gc-wasm` itself) and a `pnpm install` that has resolved `three` for
// `@gc/app` -- both preconditions `pnpm build` already has in this repo's own
// build order (see scripts/check.sh's `gate_wasm_build`, which always runs
// before the Vite build).

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

/** The subset of `cargo metadata --format-version 1`'s package shape this file reads. */
interface CargoPackage {
  readonly name: string;
  readonly version: string;
  readonly license: string | null;
  readonly manifest_path: string;
}

/** The subset of `cargo metadata --format-version 1`'s top-level shape this file reads. */
interface CargoMetadata {
  readonly packages: readonly CargoPackage[];
}

/** A third-party Rust crate linked into `gc_wasm_bg.wasm`, ready to render into a notice. */
interface WasmLinkedCrate {
  readonly name: string;
  readonly version: string;
  readonly license: string;
  readonly manifestDir: string;
}

/** The subset of a package's own `package.json` this file reads (`three`, `vite`). */
interface PackageJsonVersion {
  readonly version: string;
}

const here = dirname(fileURLToPath(import.meta.url));
const tsRootDir = join(here, "..");
const rustDir = join(tsRootDir, "..", "rust");
const rustManifestPath = join(rustDir, "Cargo.toml");

// This repository's own crates. `cargo tree` reports them as normal
// dependencies of gc-wasm too (it depends on them directly); they carry
// GOLISEO's own license, not a third-party one, so they are excluded here.
const WORKSPACE_CRATE_PREFIX = "gc-";

const DIVIDER = "=".repeat(78);

function runCargo(args: readonly string[]): string {
  try {
    return execFileSync("cargo", args, { cwd: rustDir, encoding: "utf8" });
  } catch (cause) {
    throw new Error(
      `generate_third_party_notices: \`cargo ${args.join(" ")}\` failed -- a working cargo on PATH ` +
        "is required to derive the wasm-linked crate list",
      { cause },
    );
  }
}

// Every crate normally linked into gc_wasm_bg.wasm for its shipping target,
// with the license expression cargo itself read from that crate's own
// Cargo.toml -- not a hand-maintained table. `-e no-dev,no-build,no-proc-macro`
// mirrors THIRD_PARTY.md's own audit: proc-macro crates and their
// exclusively-build-time dependents (`syn`, `quote`, `proc-macro2`,
// `bumpalo`, ...) run inside the compiler and contribute no code of their
// own to the module, so they are excluded the same way that audit excludes
// them.
function wasmLinkedCrates(): readonly WasmLinkedCrate[] {
  const metadata = JSON.parse(
    runCargo(["metadata", "--format-version", "1", "--manifest-path", rustManifestPath]),
  ) as CargoMetadata;
  const packageByKey = new Map<string, CargoPackage>(
    metadata.packages.map((pkg) => [`${pkg.name} ${pkg.version}`, pkg]),
  );

  const treeOutput = runCargo([
    "tree",
    "-p",
    "gc-wasm",
    "--target",
    "wasm32-unknown-unknown",
    "-e",
    "no-dev,no-build,no-proc-macro",
    "--prefix",
    "none",
    "--no-dedupe",
    "--manifest-path",
    rustManifestPath,
  ]);

  const seenKeys = new Set<string>();
  for (const rawLine of treeOutput.split("\n")) {
    const line = rawLine.trim();
    if (line === "") continue;
    const match = /^(\S+) v(\S+)/.exec(line);
    if (!match) {
      throw new Error(
        `generate_third_party_notices: could not parse \`cargo tree\` line: "${line}"`,
      );
    }
    const [, name, version] = match;
    if (name === undefined || version === undefined) {
      throw new Error(
        `generate_third_party_notices: \`cargo tree\` line missing name/version: "${line}"`,
      );
    }
    if (name.startsWith(WORKSPACE_CRATE_PREFIX)) continue;
    seenKeys.add(`${name} ${version}`);
  }

  const crates: WasmLinkedCrate[] = [];
  for (const key of seenKeys) {
    const pkg = packageByKey.get(key);
    if (!pkg) {
      throw new Error(
        `generate_third_party_notices: \`cargo tree\` reported "${key}" but ` +
          "`cargo metadata` has no matching package",
      );
    }
    if (pkg.license === null || pkg.license === "") {
      throw new Error(
        `generate_third_party_notices: crate "${key}" has no \`license\` field in its Cargo.toml`,
      );
    }
    crates.push({
      name: pkg.name,
      version: pkg.version,
      license: pkg.license,
      manifestDir: dirname(pkg.manifest_path),
    });
  }
  crates.sort((a, b) =>
    a.name === b.name ? a.version.localeCompare(b.version) : a.name.localeCompare(b.name),
  );
  return crates;
}

// Reads the MIT license text a crate ships, if any.
function mitLicenseText(manifestDir: string): string | null {
  for (const filename of ["LICENSE-MIT", "LICENSE-MIT.txt", "LICENSE"]) {
    const candidate = join(manifestDir, filename);
    if (existsSync(candidate)) return readFileSync(candidate, "utf8").trim();
  }
  return null;
}

// unicode-ident is the one crate in this closure whose Unicode-3.0 component
// is a separate, mandatory notice alongside its MIT/Apache option --
// THIRD_PARTY.md calls this out explicitly.
function unicodeLicenseText(manifestDir: string): string | null {
  const candidate = join(manifestDir, "LICENSE-UNICODE");
  return existsSync(candidate) ? readFileSync(candidate, "utf8").trim() : null;
}

// three.js: the only third-party runtime dependency in the JavaScript half
// of the bundle (THIRD_PARTY.md), a direct `dependencies` entry of
// `@gc/app`. Read through @gc/app's own `node_modules/three` symlink --
// pnpm creates one per direct dependency of every package -- rather than
// `three`'s own `exports` map (which does not expose `./package.json` as a
// subpath) or a pnpm-store path that changes shape with the version.
function threeJsNotice(): { readonly version: string; readonly licenseText: string } {
  const threeDir = join(tsRootDir, "packages", "app", "node_modules", "three");
  const packageJsonPath = join(threeDir, "package.json");
  if (!existsSync(packageJsonPath)) {
    throw new Error(
      `generate_third_party_notices: no \`three\` package found at ${threeDir} -- run \`pnpm install\` first`,
    );
  }
  const pkg = JSON.parse(readFileSync(packageJsonPath, "utf8")) as PackageJsonVersion;
  const licensePath = join(threeDir, "LICENSE");
  if (!existsSync(licensePath)) {
    throw new Error(
      `generate_third_party_notices: three@${pkg.version} has no LICENSE file at ${licensePath}`,
    );
  }
  return { version: pkg.version, licenseText: readFileSync(licensePath, "utf8").trim() };
}

// Vite's own module-preload polyfill: `build.modulePreload.polyfill` defaults
// to `true`, and Vite injects this small snippet (relList.supports check,
// MutationObserver fallback) into every bundle it emits -- confirmed present
// in ts/dist-app/assets/index-<hash>.js by grepping for
// `relList.supports("modulepreload")`. It ships from Vite's own source, not
// a bundled dependency, so only the "Vite core license" section of Vite's
// `LICENSE.md` applies -- the file's later "Licenses of bundled
// dependencies" section covers Vite's own build-time tooling and is not
// reproduced here.
function viteNotice(): { readonly version: string; readonly licenseText: string } {
  const viteDir = join(tsRootDir, "node_modules", "vite");
  const packageJsonPath = join(viteDir, "package.json");
  if (!existsSync(packageJsonPath)) {
    throw new Error(
      `generate_third_party_notices: no \`vite\` package found at ${viteDir} -- run \`pnpm install\` first`,
    );
  }
  const pkg = JSON.parse(readFileSync(packageJsonPath, "utf8")) as PackageJsonVersion;
  const licensePath = join(viteDir, "LICENSE.md");
  if (!existsSync(licensePath)) {
    throw new Error(
      `generate_third_party_notices: vite@${pkg.version} has no LICENSE.md file at ${licensePath}`,
    );
  }
  const fullText = readFileSync(licensePath, "utf8");
  const bundledDependenciesHeading = "\n# Licenses of bundled dependencies";
  const coreLicenseEnd = fullText.indexOf(bundledDependenciesHeading);
  if (coreLicenseEnd === -1) {
    throw new Error(
      `generate_third_party_notices: vite@${pkg.version}'s LICENSE.md no longer has a ` +
        '"Licenses of bundled dependencies" section -- the "Vite core license" extraction ' +
        "below needs re-checking against the new file shape",
    );
  }
  return { version: pkg.version, licenseText: fullText.slice(0, coreLicenseEnd).trim() };
}

function crateSection(crate: WasmLinkedCrate): string {
  const mit = mitLicenseText(crate.manifestDir);
  const unicode = unicodeLicenseText(crate.manifestDir);
  const isMultiLicense = crate.license.includes(" OR ") || crate.license.includes(" AND ");

  if (!mit) {
    // Loud on purpose, same as every other failure path in this file: a
    // silently-incomplete notices file is worse than a build that refuses to
    // produce one. If a future crate is ever taken under a non-MIT option
    // instead, this function needs a matching branch, not a placeholder line.
    throw new Error(
      `generate_third_party_notices: no LICENSE-MIT/LICENSE-MIT.txt/LICENSE file found under ` +
        `${crate.manifestDir} for crate "${crate.name} ${crate.version}" (license: "${crate.license}")`,
    );
  }

  const parts = [DIVIDER, `${crate.name} ${crate.version} -- ${crate.license}`, DIVIDER, ""];
  parts.push(
    isMultiLicense
      ? `Taken under the MIT option of this crate's "${crate.license}" license.`
      : "License text:",
  );
  parts.push("", mit);
  if (unicode) {
    parts.push(
      "",
      "This crate additionally carries a Unicode-3.0 component, reproduced below",
      "as its own separate notice:",
      "",
      unicode,
    );
  }
  return parts.join("\n");
}

/**
 * Builds the full THIRD_PARTY_NOTICES.txt content. Exported so
 * ts/vite.config.ts's build plugin can call it directly rather than
 * shelling out to this file as a subprocess.
 */
export function buildThirdPartyNotices(): string {
  const three = threeJsNotice();
  const vite = viteNotice();
  const crates = wasmLinkedCrates();

  const sections = [
    [
      "GOLISEO THIRD-PARTY NOTICES",
      "",
      "Generated by ts/scripts/generate_third_party_notices.ts during `pnpm build`",
      "(see ts/vite.config.ts) from the dependency metadata `pnpm` and `cargo`",
      "resolve at build time -- this file is not hand-maintained. It lists every",
      "third-party component actually shipped in this bundle: three.js and Vite's",
      "own module-preload polyfill in the JavaScript bundle, and every Rust crate",
      "linked into gc_wasm_bg.wasm. The wasm-bindgen-generated JS glue inlined into",
      "the bundle is covered by the wasm-bindgen crate section below -- it is",
      "wasm-bindgen's own code, not a separate component. See THIRD_PARTY.md in the",
      "repository root for the complete audited inventory, including build-only",
      "tooling that is not part of this file because it is never distributed.",
    ].join("\n"),
    [DIVIDER, `three.js ${three.version} -- MIT License`, DIVIDER, "", three.licenseText].join(
      "\n",
    ),
    [
      DIVIDER,
      `Vite ${vite.version} module-preload polyfill -- MIT License`,
      DIVIDER,
      "",
      "Vite injects this small polyfill into every bundle it builds (see",
      "build.modulePreload.polyfill). It ships from Vite's own source, so only",
      'the "Vite core license" section of its LICENSE.md applies -- reproduced',
      "below, not Vite's own build-tooling dependency notices.",
      "",
      vite.licenseText,
    ].join("\n"),
    ...crates.map(crateSection),
  ];

  return sections.join("\n\n") + "\n";
}

// Allow standalone regeneration for inspection: `node scripts/generate_third_party_notices.ts`.
if (import.meta.url === `file://${process.argv[1] ?? ""}`) {
  process.stdout.write(buildThirdPartyNotices());
}
