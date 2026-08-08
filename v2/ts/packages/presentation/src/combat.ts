// Ported from game/presentation/combat.lua.
//
// This layer only *reads* simulation state — a `MatchState` and a
// `CombatMatchState` come in, a presentation projection goes out. Nothing
// here mutates its inputs or produces anything that could flow back into
// simulation truth.
//
// Boundary note: the Lua original requires `data.action_families`,
// `data.equipment_presentations`, and `data.loadouts` as module-level
// globals. Under the v2 split, `data/**` lives in Rust (`crates/gc-data`,
// see v2/README.md §2) — there is no TypeScript module to import them from.
// The lookup tables are therefore threaded through as an explicit
// `CombatPresentationData` parameter instead of a top-level `require`,
// which keeps `@gc/presentation` free of any dependency on Rust-owned
// content while preserving the exact same projection logic.
//
// Likewise `MatchState`/`CombatMatchState`/`CombatEvent` and friends are
// declared by `sim/*.lua` (also Rust-owned, `crates/gc-sim`). Only the
// fields this module actually reads are declared locally below.

import type { Vec2 } from "@gc/core";

export type CombatPresentationReadiness =
  | "unavailable"
  | "ready"
  | "cooldown"
  | "committed"
  | "forced";

export type CombatTelegraphKind = "arc" | "guard_arc" | "line";

export type ActionFamilyId = "unarmed" | "guard" | "light_melee" | "ranged";

export type CombatActionPhase = "ready" | "windup" | "active" | "aim" | "guard" | "recovery";

export type CombatForcedState = "stagger" | "knockback";

export type CombatContactResult = "hit" | "extended" | "guarded" | "immune" | "superseded";

export type EquipmentAttachment = "left_hand" | "right_hand" | "both_hands";

export type CombatEventKind =
  | "request_rejected"
  | "commit"
  | "projectile_spawn"
  | "projectile_expire"
  | "contact"
  | "ball_spill"
  | "forced"
  | "guard_recoil"
  | "miss"
  | "interrupted"
  | "cancelled"
  | "match_terminated";

// Declared by `sim/combat.lua` in the Lua source (see the comment there: "the
// authority that emits it. Presentation only renders the closed set."). No
// TypeScript sim exists to import this from, so the closed vocabulary is
// declared here, mirroring what `data.loadouts`/`data.action_families` are
// to the other data shapes on this boundary.
export type CombatRequestRejectionReason =
  | "protected_keeper_or_no_loadout"
  | "kickoff_hold"
  | "soccer_commitment"
  | "aerial_state_or_recovery"
  | "forced_state"
  | "already_committed"
  | "cooldown"
  | "missing_press_edge"
  | "malformed_input";

export type CombatPresentationEventId =
  | "combat.commit"
  | "combat.projectile.spawn"
  | "combat.projectile.expire"
  | "combat.contact.hit"
  | "combat.contact.extended"
  | "combat.contact.guarded"
  | "combat.contact.immune"
  | "combat.contact.superseded"
  | "combat.ball_spill"
  | "combat.forced"
  | "combat.guard_recoil"
  | "combat.miss"
  | "combat.interrupted"
  | "combat.cancelled"
  | "combat.match_terminated"
  | `combat.request.rejected.${CombatRequestRejectionReason}`;

/** The authoritative combat event record, as emitted by the (Rust) sim. */
export interface CombatEvent {
  readonly kind: CombatEventKind;
  readonly tick: number;
  readonly family_id?: ActionFamilyId;
  readonly source_index?: number;
  readonly target_index?: number;
  readonly source_sequence?: number;
  readonly result?: CombatContactResult;
  readonly reason?: CombatRequestRejectionReason;
  readonly x: number;
  readonly y: number;
}

export interface CombatEventPresentation {
  readonly stable_id?: string;
  readonly semantic_id: CombatPresentationEventId;
  readonly tick: number;
  readonly family_id?: ActionFamilyId;
  readonly source_index?: number;
  readonly target_index?: number;
  readonly source_sequence?: number;
  readonly result?: CombatContactResult;
  readonly x: number;
  readonly y: number;
}

export interface CombatPlayerPresentation {
  readonly player_index: number;
  readonly player_id: string;
  readonly family_id?: ActionFamilyId;
  readonly family_name?: string;
  readonly equipment_presentation_id?: string;
  readonly equipment_name?: string;
  readonly equipment_attachment?: EquipmentAttachment;
  readonly phase: CombatActionPhase;
  readonly phase_progress: number;
  readonly phase_ticks: number;
  readonly cooldown_ticks: number;
  readonly cooldown_fraction: number;
  readonly readiness: CombatPresentationReadiness;
  readonly forced_state?: CombatForcedState;
  readonly forced_ticks: number;
  readonly immunity_ticks: number;
  readonly position: Vec2;
  readonly direction: Vec2;
  readonly reach_px?: number;
  readonly projectile_range_px?: number;
  readonly front_arc_degrees?: number;
  readonly telegraph_kind?: CombatTelegraphKind;
  readonly source_sequence?: number;
}

export interface CombatProjectilePresentation {
  readonly family_id: ActionFamilyId;
  readonly source_index: number;
  readonly source_sequence: number;
  readonly position: Vec2;
  readonly direction: Vec2;
  readonly remaining_ticks: number;
}

export interface CombatPresentationModel {
  readonly enabled: boolean;
  readonly tick?: number;
  readonly players: readonly CombatPlayerPresentation[];
  readonly projectiles: readonly CombatProjectilePresentation[];
}

// --- Sim-shaped inputs (the slice presentation actually reads) -----------

export interface MatchPlayerState {
  readonly id: string;
  readonly pos: Vec2;
  readonly facing: Vec2;
}

export interface MatchState {
  readonly players: readonly MatchPlayerState[];
}

export interface CombatPlayerState {
  readonly loadout_id?: string;
  readonly family_id?: ActionFamilyId;
  readonly phase: CombatActionPhase;
  readonly phase_ticks: number;
  readonly cooldown_ticks: number;
  readonly source_sequence?: number;
  readonly forced_state?: CombatForcedState;
  readonly forced_ticks: number;
  readonly immunity_ticks: number;
}

export interface CombatProjectile {
  readonly family_id: ActionFamilyId;
  readonly source_index: number;
  readonly source_sequence: number;
  readonly pos: Vec2;
  readonly dir: Vec2;
  readonly remaining_ticks: number;
}

export interface CombatMatchState {
  readonly tick?: number;
  readonly players: readonly CombatPlayerState[];
  readonly player_ids: readonly string[];
  readonly projectiles: readonly CombatProjectile[];
}

// --- Data lookups (owned by `crates/gc-data`, injected rather than required) --

export interface ActionFamilyData {
  readonly id: ActionFamilyId;
  readonly name: string;
  readonly windup_ticks: number;
  readonly active_ticks?: number;
  readonly recovery_ticks: number;
  readonly cooldown_ticks: number;
  readonly reach_px?: number;
  readonly projectile_speed_px_per_second?: number;
  readonly projectile_lifetime_ticks?: number;
  readonly front_arc_degrees: number;
}

export interface EquipmentPresentationData {
  readonly id: string;
  readonly name: string;
  readonly family_id: ActionFamilyId;
  readonly attachment: EquipmentAttachment;
}

export interface FixedLoadoutData {
  readonly id: string;
  readonly family_id: ActionFamilyId;
  readonly equipment_presentation_id: string;
}

export interface CombatPresentationData {
  readonly action_families: Readonly<Record<ActionFamilyId, ActionFamilyData>>;
  readonly equipment_presentations: Readonly<Record<string, EquipmentPresentationData>>;
  readonly loadouts: Readonly<Record<string, FixedLoadoutData>>;
}

// --- helpers ---------------------------------------------------------------

function invariant(condition: boolean, message: string): asserts condition {
  if (!condition) {
    throw new Error(message);
  }
}

function clamp01(value: number): number {
  return Math.max(0, Math.min(1, value));
}

function phaseProgress(runtime: CombatPlayerState, family: ActionFamilyData | undefined): number {
  if (family === undefined) {
    return 0;
  }
  let duration: number;
  if (runtime.phase === "windup") {
    duration = family.windup_ticks;
  } else if (runtime.phase === "active") {
    duration = family.active_ticks ?? 1;
  } else if (runtime.phase === "recovery") {
    duration = family.recovery_ticks;
  } else {
    return runtime.phase === "ready" ? 0 : 1;
  }
  return clamp01(1 - runtime.phase_ticks / Math.max(1, duration));
}

function readiness(runtime: CombatPlayerState): CombatPresentationReadiness {
  if (runtime.family_id === undefined) {
    return "unavailable";
  } else if (runtime.forced_ticks > 0) {
    return "forced";
  } else if (runtime.phase !== "ready") {
    return "committed";
  } else if (runtime.cooldown_ticks > 0) {
    return "cooldown";
  }
  return "ready";
}

function telegraphKind(
  runtime: CombatPlayerState,
  family: ActionFamilyData | undefined
): CombatTelegraphKind | undefined {
  if (family === undefined || runtime.forced_ticks > 0) {
    return undefined;
  }
  if (family.id === "guard" && (runtime.phase === "windup" || runtime.phase === "guard")) {
    return "guard_arc";
  } else if (
    family.id === "ranged" &&
    (runtime.phase === "windup" || runtime.phase === "aim" || runtime.phase === "active")
  ) {
    return "line";
  } else if (
    (family.id === "unarmed" || family.id === "light_melee") &&
    (runtime.phase === "windup" || runtime.phase === "active")
  ) {
    return "arc";
  }
  return undefined;
}

/** Drops keys whose value is `undefined`, honestly typed as `Partial<T>`. */
function definedFields<T extends object>(fields: { [K in keyof T]?: T[K] | undefined }): Partial<T> {
  const result: Partial<T> = {};
  for (const key of Object.keys(fields) as (keyof T)[]) {
    const value = fields[key];
    if (value !== undefined) {
      result[key] = value;
    }
  }
  return result;
}

// --- module ------------------------------------------------------------

function model(
  state: MatchState,
  combatState: CombatMatchState | null,
  data: CombatPresentationData
): CombatPresentationModel {
  if (combatState === null) {
    return { enabled: false, players: [], projectiles: [] };
  }
  invariant(
    combatState.players.length === state.players.length,
    "combat presentation player count mismatch"
  );
  invariant(
    combatState.player_ids.length === state.players.length,
    "combat presentation identity mismatch"
  );

  const players: CombatPlayerPresentation[] = state.players.map((player, i) => {
    const playerIndex = i + 1;
    const playerId = combatState.player_ids[i];
    invariant(playerId === player.id, "combat presentation player identity mismatch");
    const runtime = combatState.players[i];
    invariant(runtime !== undefined, "combat presentation runtime is missing");

    const family = runtime.family_id !== undefined ? data.action_families[runtime.family_id] : undefined;
    const loadout = runtime.loadout_id !== undefined ? data.loadouts[runtime.loadout_id] : undefined;
    const equipment =
      loadout !== undefined ? data.equipment_presentations[loadout.equipment_presentation_id] : undefined;

    let projectileRangePx: number | undefined;
    if (
      family !== undefined &&
      family.projectile_speed_px_per_second !== undefined &&
      family.projectile_lifetime_ticks !== undefined
    ) {
      projectileRangePx = (family.projectile_speed_px_per_second * family.projectile_lifetime_ticks) / 60;
    }

    const cooldownFraction =
      family !== undefined && family.cooldown_ticks > 0
        ? clamp01(runtime.cooldown_ticks / family.cooldown_ticks)
        : 0;

    return {
      player_index: playerIndex,
      player_id: player.id,
      phase: runtime.phase,
      phase_progress: phaseProgress(runtime, family),
      phase_ticks: runtime.phase_ticks,
      cooldown_ticks: runtime.cooldown_ticks,
      cooldown_fraction: cooldownFraction,
      readiness: readiness(runtime),
      forced_ticks: runtime.forced_ticks,
      immunity_ticks: runtime.immunity_ticks,
      position: player.pos,
      direction: player.facing,
      ...definedFields<CombatPlayerPresentation>({
        family_id: runtime.family_id,
        family_name: family?.name,
        equipment_presentation_id: equipment?.id,
        equipment_name: equipment?.name,
        equipment_attachment: equipment?.attachment,
        forced_state: runtime.forced_state,
        reach_px: family?.reach_px,
        projectile_range_px: projectileRangePx,
        front_arc_degrees: family?.front_arc_degrees,
        telegraph_kind: telegraphKind(runtime, family),
        source_sequence: runtime.source_sequence,
      }),
    };
  });

  const projectiles: CombatProjectilePresentation[] = combatState.projectiles.map((projectile) => ({
    family_id: projectile.family_id,
    source_index: projectile.source_index,
    source_sequence: projectile.source_sequence,
    position: projectile.pos,
    direction: projectile.dir,
    remaining_ticks: projectile.remaining_ticks,
  }));

  return {
    enabled: true,
    ...(combatState.tick !== undefined ? { tick: combatState.tick } : {}),
    players,
    projectiles,
  };
}

function eventSemanticId(evt: CombatEvent): CombatPresentationEventId {
  if (evt.kind === "projectile_spawn") {
    return "combat.projectile.spawn";
  } else if (evt.kind === "projectile_expire") {
    return "combat.projectile.expire";
  } else if (evt.kind === "contact") {
    invariant(evt.result !== undefined, "combat contact result is required");
    return `combat.contact.${evt.result}`;
  } else if (evt.kind === "request_rejected") {
    invariant(evt.reason !== undefined, "a rejected combat request requires its typed reason");
    return `combat.request.rejected.${evt.reason}`;
  }
  return `combat.${evt.kind}` as CombatPresentationEventId;
}

function event(evt: CombatEvent, stableId?: string): CombatEventPresentation {
  return {
    semantic_id: eventSemanticId(evt),
    tick: evt.tick,
    x: evt.x,
    y: evt.y,
    ...definedFields<CombatEventPresentation>({
      stable_id: stableId,
      family_id: evt.family_id,
      source_index: evt.source_index,
      target_index: evt.target_index,
      source_sequence: evt.source_sequence,
      result: evt.result,
    }),
  };
}

export const combat = { model, event };
