// The headless entry point for the OMP-3 fault harness.
//
// It prints one line per fact, in a fixed order, so two runs -- or two
// *separate operating-system processes* -- can be compared with a plain
// text diff and a digest. That is the whole reason this is a marker stream
// rather than a human-readable log: `scripts/fault_harness.py` compares
// processes, and a comparison is only as good as the determinism of what it
// compares.
//
// ## Boundary note: `game.online.fault_scenarios`
//
// `game.online.fault_scenarios` (and the `fault_harness` it drives) is
// Rust-owned (`crates/gc-netcode`, ARCHITECTURE.md §1.1) -- this campaign
// controller only selects rows, runs them, and formats the result. The
// scenario table and the `run` function are therefore threaded through as
// an injected `FaultScenariosPort` rather than required, following the same
// pattern `@gc/presentation/src/combat.ts` uses for content tables. `print`
// is injected too, defaulting to `console.log`, so a spec can capture the
// marker stream instead of it landing on stdout.
//
// ## Why the hash-order probe is now a constant
//
// `hashOrderProbe` exists so a campaign controller comparing two processes
// can *demonstrate* they really used different hash seeds, rather than
// assume it. The idea: a per-process randomized iteration order over a
// fixed table is exactly what would let a same-process multi-client harness
// fail to catch a hash-order-induced divergence, since two clients sharing
// one process would otherwise share a seed and agree by accident. The probe
// emits this process's observed iteration order over `PROBE_KEYS` so two
// runs can be diffed.
//
// JavaScript objects and `Map`s have no such randomization: `Object.keys()`,
// `for...in`, and `Map` iteration are all specified to preserve insertion
// order deterministically, with no per-process hash-seed randomization.
// There is therefore no way for this probe to serve its original purpose
// here -- two Node processes running this function will always report the
// same order, every time, which is the opposite of what the check is for.
// `hashOrderProbe` is kept for interface parity (and because the marker
// stream's shape is otherwise unchanged), but it is a constant, not a
// probe, in this implementation.

export type FaultHarnessTopology = "star" | "relay";

/** `game.online.match_manifest`'s `SessionMatchMode` (Rust-owned). */
export type SessionMatchMode = "1v1" | "2v2" | "4v4";

/** `game.online.fault_scenarios`'s `FaultScenario` (Rust-owned; only the fields this module reads). */
export interface FaultScenario {
  readonly id: string;
  readonly known_gap?: string;
}

/** `game.online.fault_harness`'s `FaultHarnessFinding` (Rust-owned). */
export interface FaultFinding {
  readonly id: string;
  readonly ok: boolean;
  readonly skipped: boolean;
  readonly detail: string;
}

/** `game.online.fault_harness`'s `FaultHarnessReport` (Rust-owned). */
export interface FaultReport {
  readonly markers: readonly string[];
  readonly notes: readonly string[];
  readonly findings: readonly FaultFinding[];
  readonly ok: boolean;
}

export interface FaultRunOptions {
  readonly network_seed?: number;
  readonly duration_ticks?: number;
  readonly topology: FaultHarnessTopology;
}

/**
 * `game.online.fault_scenarios`, injected -- see the module doc comment.
 * `SCENARIOS` is the full declared table (used only to name every known
 * gap, selected or not); `select`/`find`/`run` mirror
 * `game.online.fault_scenarios`'s functions of the same name.
 */
export interface FaultScenariosPort {
  readonly SCENARIOS: readonly FaultScenario[];
  select(smokeOnly: boolean): readonly FaultScenario[];
  find(id: string): FaultScenario | null;
  run(scenario: FaultScenario, options: FaultRunOptions): FaultReport;
}

export const MARKER_VERSION = 1;

// A fixed table with enough string keys that a per-process hash seed would
// reorder it with overwhelming probability, in a runtime that randomizes
// iteration order. It is never used for anything else. Kept for interface
// parity; see the header comment above for why the probe itself is now a
// constant.
export const PROBE_KEYS: readonly string[] = [
  "alpha",
  "bravo",
  "charlie",
  "delta",
  "echo",
  "foxtrot",
  "golf",
  "hotel",
  "india",
  "juliet",
  "kilo",
  "lima",
];

export function hashOrderProbe(): string {
  return PROBE_KEYS.join(",");
}

function selectRows(
  scenarios: FaultScenariosPort,
  selector: string | undefined
): { readonly rows: readonly FaultScenario[]; readonly smokeOnly: boolean; readonly label: string } {
  if (selector === undefined || selector === "smoke") {
    return { rows: scenarios.select(true), smokeOnly: true, label: "smoke" };
  }
  if (selector === "full") {
    return { rows: scenarios.select(false), smokeOnly: false, label: "full" };
  }
  const scenario = scenarios.find(selector);
  if (scenario === null) {
    throw new Error(`unknown fault harness scenario: ${selector}`);
  }
  return { rows: [scenario], smokeOnly: false, label: selector };
}

export interface FaultCampaignOptions {
  readonly selector?: string;
  readonly network_seed?: number;
  readonly duration_ticks?: number;
  readonly topology?: FaultHarnessTopology;
  readonly print?: (line: string) => void;
}

// Run the selected rows and print the marker stream. Returns `false` when
// any non-skipped finding failed.
export function main(scenarios: FaultScenariosPort, options: FaultCampaignOptions = {}): boolean {
  const print = options.print ?? ((line: string) => {
    // eslint-disable-next-line no-console
    console.log(line);
  });
  // A crash escaping unprotected used to hang a headless run rather than
  // fail it, because there was no way to dismiss the runtime's own error
  // screen without a window. Nothing here has that problem, but the same
  // discipline holds -- every row is protected, and a crash is reported as
  // a failing row and a `false` return rather than an uncaught exception.
  try {
    return runSelection(scenarios, options, print);
  } catch (error) {
    print("RESULT fail rows=0 failures=1");
    print(`error ${String(error)}`);
    return false;
  }
}

export function runSelection(
  scenarios: FaultScenariosPort,
  options: FaultCampaignOptions,
  print: (line: string) => void
): boolean {
  const { rows, smokeOnly, label } = selectRows(scenarios, options.selector);
  const topology = options.topology ?? "star";
  print(`FAULT-HARNESS v${MARKER_VERSION}`);
  print(`selection ${label} rows=${rows.length} topology=${topology}`);
  print(`hash-order-probe ${hashOrderProbe()}`);
  if (smokeOnly) {
    // Logged, never implied: the CI subset is a subset.
    print(
      `note bounded coverage: the smoke selection runs ${rows.length} of ${scenarios.SCENARIOS.length} declared rows; ` +
        "run `--fault-harness full` for the complete matrix"
    );
  }
  // Named in every run, selected or not: a gap the campaign already knows
  // about is evidence, and hiding it behind an unselected row would be the
  // silent truncation this harness exists to refuse.
  for (const scenario of scenarios.SCENARIOS) {
    if (scenario.known_gap !== undefined) {
      print(`known-gap ${scenario.id} ${scenario.known_gap}`);
    }
  }
  let failures = 0;
  for (const scenario of rows) {
    print(`scenario ${scenario.id}`);
    let report: FaultReport;
    try {
      report = scenarios.run(scenario, {
        ...(options.network_seed !== undefined ? { network_seed: options.network_seed } : {}),
        ...(options.duration_ticks !== undefined ? { duration_ticks: options.duration_ticks } : {}),
        topology,
      });
    } catch (error) {
      print(`finding ${scenario.id} scenario.crashed FAIL ${String(error)}`);
      print(`scenario-result ${scenario.id} FAIL`);
      failures += 1;
      continue;
    }
    for (const marker of report.markers) {
      print(`marker ${marker}`);
    }
    for (const note of report.notes) {
      print(`note ${note}`);
    }
    for (const entry of report.findings) {
      const verdict = entry.skipped ? "SKIP" : entry.ok ? "ok" : "FAIL";
      print(`finding ${scenario.id} ${entry.id} ${verdict} ${entry.detail}`);
      if (!entry.ok) {
        failures += 1;
      }
    }
    print(`scenario-result ${scenario.id} ${report.ok ? "ok" : "FAIL"}`);
  }
  print(`RESULT ${failures === 0 ? "ok" : "fail"} rows=${rows.length} failures=${failures}`);
  return failures === 0;
}
