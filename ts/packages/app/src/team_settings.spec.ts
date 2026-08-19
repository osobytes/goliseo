// Coverage for `team_settings.ts` only: the storage-layer round trip
// (`serialize`/`parse`/`load`/`save`, tolerant of missing/corrupt input,
// mirroring `settings.spec.ts`) and the separate, pure content-validation
// step (`validateAgainstContent`).

import { describe, expect, it } from "vitest";
import { ok, type Result } from "@gc/core";
import { teamSettings, type TeamPreferences } from "./team_settings.ts";
import type { SettingsStorage } from "./settings.ts";
import { MATCH_CONTRACT_CONTENT, NEBULA } from "./test_support/fixtures.ts";

function memoryStorage(): SettingsStorage & { contents: string | undefined } {
  const box: { contents: string | undefined } = { contents: undefined };
  return {
    get contents() {
      return box.contents;
    },
    read: () => box.contents,
    write: (value): Result<true, string> => {
      box.contents = value;
      return ok(true);
    },
  };
}

describe("team settings storage", () => {
  it("round trips a full set of preferences through deterministic storage", () => {
    const storage = memoryStorage();
    const value: TeamPreferences = {
      starterIds: ["ozzo", "brakka", "veil_nyx", "rok_tann", "zyro_vex"],
      formationId: "1-1-2",
      tacticId: "press_high",
      combatEnabled: false,
      lastOnlineMode: "2v2",
      lastBotFill: true,
    };
    const saved = teamSettings.save(value, storage);
    expect(saved.ok).toBe(true);
    expect(storage.contents?.startsWith("version=1")).toBe(true);

    const loaded = teamSettings.load(storage);
    expect(loaded).toEqual(value);
  });

  it("falls back to defaults when storage is missing entirely", () => {
    expect(teamSettings.load(undefined)).toEqual(teamSettings.defaults());
    const saved = teamSettings.save(teamSettings.defaults(), undefined);
    expect(saved.ok).toBe(false);
  });

  it("falls back to defaults when the stored text is absent", () => {
    const storage: SettingsStorage = { read: () => undefined, write: () => ok(true) };
    expect(teamSettings.load(storage)).toEqual(teamSettings.defaults());
  });

  it("never throws on corrupt stored text, and falls back field by field", () => {
    const storage: SettingsStorage = {
      read: () => "not even close to the wire format\0\0\0garbage",
      write: () => ok(true),
    };
    expect(() => teamSettings.load(storage)).not.toThrow();
    expect(teamSettings.load(storage)).toEqual(teamSettings.defaults());

    // A partially valid blob keeps what parses and defaults the rest.
    const partial: SettingsStorage = {
      read: () => "version=1\nformation_id=2-1-1\ncombat_enabled=not-a-boolean\n",
      write: () => ok(true),
    };
    const loaded = teamSettings.load(partial);
    expect(loaded.formationId).toBe("2-1-1");
    expect(loaded.combatEnabled).toBe(true); // invalid boolean text falls back to the default
    expect(loaded.starterIds).toEqual([]);
  });

  it("validates invalid types out of a hand-built raw record", () => {
    const value = teamSettings.validate({
      starterIds: "not-an-array",
      formationId: 42,
      combatEnabled: "yes",
    });
    expect(value.starterIds).toEqual([]);
    expect(value.formationId).toBe("");
    expect(value.combatEnabled).toBe(true);
  });
});

describe("team preferences validated against content", () => {
  const stored: TeamPreferences = {
    starterIds: ["ozzo", "brakka", "veil_nyx", "rok_tann", "zyro_vex"],
    formationId: "1-1-2",
    tacticId: "press_high",
    combatEnabled: false,
    lastOnlineMode: "4v4",
    lastBotFill: true,
  };

  it("keeps every stored id that still exists in the content tables", () => {
    const resolved = teamSettings.validateAgainstContent(MATCH_CONTRACT_CONTENT, NEBULA, stored);
    expect(resolved.starterIds).toEqual(stored.starterIds);
    expect(resolved.formationId).toBe("1-1-2");
    expect(resolved.tacticId).toBe("press_high");
    expect(resolved.combatEnabled).toBe(false);
  });

  it("falls back to the home team's defaults when a formation no longer exists", () => {
    const resolved = teamSettings.validateAgainstContent(MATCH_CONTRACT_CONTENT, NEBULA, {
      ...stored,
      formationId: "retired-shape",
    });
    expect(resolved.formationId).toBe(NEBULA.formation);
  });

  it("falls back to balanced when a tactic no longer exists", () => {
    const resolved = teamSettings.validateAgainstContent(MATCH_CONTRACT_CONTENT, NEBULA, {
      ...stored,
      tacticId: "retired-plan",
    });
    expect(resolved.tacticId).toBe("balanced");
  });

  it("falls back to the roster default when a stored starter no longer exists", () => {
    const resolved = teamSettings.validateAgainstContent(MATCH_CONTRACT_CONTENT, NEBULA, {
      ...stored,
      starterIds: ["ozzo", "brakka", "veil_nyx", "rok_tann", "someone_deleted"],
    });
    expect(resolved.starterIds).toEqual(NEBULA.roster);
  });

  it("falls back when the stored five is otherwise no longer legal (two keepers)", () => {
    const resolved = teamSettings.validateAgainstContent(MATCH_CONTRACT_CONTENT, NEBULA, {
      ...stored,
      starterIds: ["ozzo", "ozzo", "brakka", "veil_nyx", "rok_tann"],
    });
    expect(resolved.starterIds).toEqual(NEBULA.roster);
  });

  it("falls back to the roster default when nothing was ever stored", () => {
    const resolved = teamSettings.validateAgainstContent(
      MATCH_CONTRACT_CONTENT,
      NEBULA,
      teamSettings.defaults(),
    );
    expect(resolved.starterIds).toEqual(NEBULA.roster);
    expect(resolved.formationId).toBe(NEBULA.formation);
    expect(resolved.tacticId).toBe("balanced");
  });
});
