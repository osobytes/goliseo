// Persists the player's team sheet (starters, formation, tactic, combat
// toggle) and the host's last online lobby choices (match mode, bot fill),
// following `settings.ts`'s exact discipline: a versioned key-value text
// blob, tolerant of missing or corrupt stored text (falls back to defaults,
// never throws). `settings.ts`'s own header explains why a browser has no
// filesystem persistence this milestone -- `SettingsStorage` (reused here
// rather than redeclared) is the same explicit-port answer to that gap.
//
// Two separate concerns live in this one file on purpose, mirroring the two
// steps `settings.ts` collapses into one because it never needs the second:
//
//   1. `defaults`/`validate`/`serialize`/`parse`/`load`/`save` -- the
//      storage-layer round trip. It knows nothing about content; a stored id
//      is just a string here.
//   2. `validateAgainstContent` -- a separate, PURE step that checks a
//      stored id against the CURRENT content tables and falls back to the
//      home team's own default when it no longer exists. AGENTS.md §8:
//      content is data and may drift out from under a value saved on an
//      earlier visit (a formation renamed, a tactic removed).
//
// `lastOnlineMode` is stored as a plain string rather than `@gc/screens`'s
// `SessionMatchMode` so this module keeps no dependency on that package;
// the caller (`app.ts`, which already depends on `@gc/screens`) is
// responsible for checking it against `LOBBY_MODES`.

import { type Result, err } from "@gc/core";
import type { SettingsStorage } from "./settings.ts";
import type { MatchContractContent, TeamData } from "./content.ts";
import { validateStarters } from "./match_contract.ts";

/** What this module persists across a reload. */
export interface TeamPreferences {
  readonly starterIds: readonly string[];
  readonly formationId: string;
  readonly tacticId: string;
  readonly combatEnabled: boolean;
  /** Last chosen match-mode id ("1v1"/"2v2"/"4v4"), or `""` if never set. */
  readonly lastOnlineMode: string;
  readonly lastBotFill: boolean;
}

function defaults(): TeamPreferences {
  return {
    starterIds: [],
    formationId: "",
    tacticId: "",
    combatEnabled: true,
    lastOnlineMode: "",
    lastBotFill: false,
  };
}

function stringArrayOr(value: unknown, fallback: readonly string[]): readonly string[] {
  if (!Array.isArray(value) || !value.every((entry) => typeof entry === "string")) {
    return fallback;
  }
  return value;
}

function validate(
  input: Partial<Record<keyof TeamPreferences, unknown>> | undefined,
): TeamPreferences {
  const raw = input ?? {};
  const base = defaults();
  const stringOr = (key: keyof TeamPreferences): string => {
    const value = raw[key];
    return typeof value === "string" ? value : (base[key] as string);
  };
  const booleanOr = (key: keyof TeamPreferences): boolean => {
    const value = raw[key];
    return typeof value === "boolean" ? value : (base[key] as boolean);
  };
  return {
    starterIds: stringArrayOr(raw.starterIds, base.starterIds),
    formationId: stringOr("formationId"),
    tacticId: stringOr("tacticId"),
    combatEnabled: booleanOr("combatEnabled"),
    lastOnlineMode: stringOr("lastOnlineMode"),
    lastBotFill: booleanOr("lastBotFill"),
  };
}

function boolString(value: boolean): string {
  return value ? "true" : "false";
}

function serialize(value: TeamPreferences): string {
  const clean = validate(value);
  return [
    "version=1",
    `starter_ids=${clean.starterIds.join(",")}`,
    `formation_id=${clean.formationId}`,
    `tactic_id=${clean.tacticId}`,
    `combat_enabled=${boolString(clean.combatEnabled)}`,
    `last_online_mode=${clean.lastOnlineMode}`,
    `last_bot_fill=${boolString(clean.lastBotFill)}`,
    "",
  ].join("\n");
}

/** Wire key (as written by `serialize`) -> the `TeamPreferences` field it fills. */
const FIELD_FOR_KEY: Readonly<Record<string, keyof TeamPreferences>> = {
  starter_ids: "starterIds",
  formation_id: "formationId",
  tactic_id: "tacticId",
  combat_enabled: "combatEnabled",
  last_online_mode: "lastOnlineMode",
  last_bot_fill: "lastBotFill",
};

const BOOLEAN_FIELDS: ReadonlySet<keyof TeamPreferences> = new Set([
  "combatEnabled",
  "lastBotFill",
]);

function parse(contents: string): TeamPreferences {
  const raw: Partial<Record<keyof TeamPreferences, unknown>> = {};
  for (const match of contents.matchAll(/(\w+)=([^\r\n]*)/g)) {
    const key = match[1];
    const value = match[2];
    if (key === undefined || value === undefined) {
      continue;
    }
    const field = FIELD_FOR_KEY[key];
    if (field === undefined) {
      continue;
    }
    if (field === "starterIds") {
      raw[field] = value === "" ? [] : value.split(",");
    } else if (BOOLEAN_FIELDS.has(field)) {
      // Mirrors `settings.ts`'s own `parse`: only an exact "true"/"false"
      // is a value at all -- anything else (hand-edited storage, a future
      // format) is left unset here, so `validate` falls back to the
      // default rather than silently accepting `false` for garbage text.
      if (value === "true") {
        raw[field] = true;
      } else if (value === "false") {
        raw[field] = false;
      }
    } else {
      raw[field] = value;
    }
  }
  return validate(raw);
}

function load(storage: SettingsStorage | undefined): TeamPreferences {
  if (!storage) {
    return defaults();
  }
  const contents = storage.read();
  return contents !== undefined ? parse(contents) : defaults();
}

function save(value: TeamPreferences, storage: SettingsStorage | undefined): Result<true, string> {
  if (!storage) {
    return err("team settings storage is unavailable");
  }
  return storage.write(serialize(value));
}

/**
 * Resolves stored starter/formation/tactic ids against the CURRENT content
 * tables, falling back to the home team's own defaults for anything that no
 * longer validates -- an empty stored value (never saved), an id the
 * content tables no longer carry, or a starter five that no longer forms a
 * legal team sheet (AGENTS.md §8; `match_contract.ts`'s `validateStarters`
 * is the same check the team sheet itself enforces before it will commit).
 * Pure: takes the already-loaded preferences, does no storage I/O.
 */
function validateAgainstContent(
  content: Pick<MatchContractContent, "players" | "formations" | "tactics">,
  homeTeam: TeamData,
  stored: TeamPreferences,
): TeamPreferences {
  const formationId =
    stored.formationId !== "" && content.formations[stored.formationId] !== undefined
      ? stored.formationId
      : homeTeam.formation;
  const tacticId =
    stored.tacticId !== "" && content.tactics[stored.tacticId] !== undefined
      ? stored.tacticId
      : "balanced";
  const startersValid =
    stored.starterIds.length > 0 && validateStarters(content, stored.starterIds, homeTeam).ok;
  return {
    ...stored,
    starterIds: startersValid ? [...stored.starterIds] : [...homeTeam.roster],
    formationId,
    tacticId,
  };
}

export const teamSettings = {
  defaults,
  validate,
  serialize,
  parse,
  load,
  save,
  validateAgainstContent,
};
