// Ported from game/match_hud.lua.
//
// The HUD reads the scoreboard section of the same versioned render payload
// the pitch draws from, never a `MatchState`. `RenderFrameHud` is
// `render/frame.lua`'s (Rust-owned, `render/**` -> `crates/gc-render`;
// v2/README.md §2), declared locally per rule 6.7 -- only the fields this
// module reads. `render/identity.lua`'s `PlayerPresentationIdentity` is
// TypeScript-owned (`@gc/render`) but not yet ported there either (this
// package's porting report), so `identity.for_player` is injected as an
// `IdentityPort`. `CombatPlayerPresentation`/`CombatFeedbackNotice` are
// `@gc/presentation`'s (not a declared dependency -- report per the task
// brief); declared locally, structurally compatible, same as `content.ts`
// patterns elsewhere in this port.

export type RenderSpeciesShape = "round" | "broad" | "angular" | "cluster";
export type RenderTeam = "home" | "away";
export type BroadcastPhase = "kickoff" | "goal" | "replay" | "full_time";

/** The slice of `render/frame.lua`'s `RenderFrameHud` this module reads. */
export interface RenderFrameHud {
  readonly home_score: number;
  readonly away_score: number;
  readonly time_left: number;
  readonly possession_team?: RenderTeam;
  readonly controlled_id: string;
  readonly controlled_team: RenderTeam;
  readonly controlled_is_keeper: boolean;
  readonly controlled_owns_ball: boolean;
  readonly controlled_stamina: number;
  readonly species_shape: RenderSpeciesShape;
  readonly species_color: readonly number[];
}

/** The slice of `@gc/presentation`'s `CombatPlayerPresentation` this module reads. */
export interface CombatPlayerPresentation {
  readonly family_name?: string;
  readonly readiness: "unavailable" | "ready" | "cooldown" | "forced" | "committed";
  readonly cooldown_fraction: number;
  readonly phase: string;
  readonly phase_progress: number;
}

/** The slice of `@gc/presentation`'s `CombatFeedbackNotice` this module reads. */
export interface CombatFeedbackNotice {
  readonly text: string;
  readonly glyph: string;
}

export interface MatchHudContext {
  readonly home_name: string;
  readonly away_name: string;
  readonly arena_name: string;
  readonly arena_location: string;
  readonly tactic_name: string;
  readonly formation_name: string;
  readonly prompt?: OnboardingPromptLike;
  readonly phase?: BroadcastPhase;
  readonly scoring_team?: RenderTeam;
  readonly combat_enabled?: boolean;
  readonly combat?: CombatPlayerPresentation;
  readonly combat_notice?: CombatFeedbackNotice;
}

/** `game/match_onboarding.lua`'s `OnboardingPrompt` -- see match_onboarding.ts. */
export interface OnboardingPromptLike {
  readonly id: string;
  readonly title: string;
  readonly body: string;
}

export interface MatchHudModel {
  readonly home_name: string;
  readonly away_name: string;
  readonly home_score: number;
  readonly away_score: number;
  readonly clock: string;
  readonly venue: string;
  readonly possession: string;
  readonly possession_marker: "filled" | "outline";
  readonly player_name: string;
  readonly player_detail: string;
  readonly player_state: string;
  readonly species_shape: RenderSpeciesShape;
  readonly species_color: readonly number[];
  readonly stamina: number;
  readonly plan: string;
  readonly prompt?: OnboardingPromptLike;
  readonly announcement_title?: string;
  readonly announcement_detail?: string;
  readonly announcement_kind?: BroadcastPhase;
  readonly equipment_label?: string;
  readonly equipment_state?: string;
  readonly equipment_progress: number;
  readonly feedback_text?: string;
  readonly feedback_glyph?: string;
}

export interface Rect {
  readonly x: number;
  readonly y: number;
  readonly w: number;
  readonly h: number;
}

export interface MatchHudLayout {
  readonly venue: Rect;
  readonly scorebug: Rect;
  readonly clock: Rect;
  readonly status: Rect;
  readonly identity: Rect;
  readonly plan: Rect;
  readonly prompt: Rect;
  readonly announcement: Rect;
  readonly combat: Rect;
  readonly scale: number;
}

/** `render/identity.lua`'s `identity.for_player`, injected -- see this file's header. */
export interface PlayerPresentationIdentity {
  readonly name: string;
  readonly species_name: string;
  readonly position: string;
}

export interface IdentityPort {
  forPlayer(playerId: string): PlayerPresentationIdentity | undefined;
}

function clamp01(value: number): number {
  return Math.max(0, Math.min(1, value));
}

export function formatClock(seconds: number): string {
  const whole = Math.max(0, Math.floor(seconds));
  const minutes = Math.floor(whole / 60);
  const secs = whole % 60;
  return `${minutes}:${secs.toString().padStart(2, "0")}`;
}

// The HUD reads the scoreboard section of the same versioned render payload the
// pitch draws from, never a `MatchState`. Only authored match metadata (team
// names, arena, tactic, onboarding) comes in as context: none of it is
// simulation state.
function model(identity: IdentityPort, scoreboard: RenderFrameHud, context: MatchHudContext): MatchHudModel {
  const presentation = identity.forPlayer(scoreboard.controlled_id);
  if (!presentation) {
    throw new Error(`missing presentation identity for ${scoreboard.controlled_id}`);
  }
  const ownerTeam = scoreboard.possession_team;
  let possession = "LOOSE BALL";
  if (ownerTeam !== undefined) {
    possession = `${ownerTeam === "home" ? context.home_name : context.away_name} POSSESSION`;
  }

  let playerState = "DEFENDING";
  if (scoreboard.controlled_owns_ball) {
    playerState = scoreboard.controlled_is_keeper ? "KEEPER BALL" : "ON BALL";
  } else if (ownerTeam === undefined) {
    playerState = "CONTESTING";
  }

  let title: string | undefined;
  let detail: string | undefined;
  if (context.phase === "kickoff") {
    title = context.combat_enabled ? "COMBAT PROTOTYPE" : "SHOWCASE FIXTURE";
    detail = `${context.home_name}  ·  ${context.away_name}`;
  } else if (context.phase === "goal") {
    const teamName = context.scoring_team === "away" ? context.away_name : context.home_name;
    title = `GOAL · ${teamName.toUpperCase()}`;
    detail = `${scoreboard.home_score}  —  ${scoreboard.away_score}`;
  } else if (context.phase === "replay") {
    title = "REPLAY";
    detail = "[A / SPACE] SKIP";
  } else if (context.phase === "full_time") {
    title = "FULL TIME";
    detail = `${context.home_name.toUpperCase()}  ${scoreboard.home_score} — ${scoreboard.away_score}  ${context.away_name.toUpperCase()}`;
  }

  let equipmentLabel: string | undefined;
  let equipmentState: string | undefined;
  let equipmentProgress = 0;
  if (context.combat && context.combat.family_name !== undefined) {
    const combat = context.combat;
    equipmentLabel = combat.family_name?.toUpperCase();
    if (combat.readiness === "ready") {
      equipmentState = "READY";
      equipmentProgress = 1;
    } else if (combat.readiness === "cooldown") {
      equipmentState = "COOLDOWN";
      equipmentProgress = 1 - combat.cooldown_fraction;
    } else if (combat.readiness === "forced") {
      equipmentState = "INTERRUPTED";
      equipmentProgress = 0;
    } else if (combat.readiness === "committed") {
      equipmentState = combat.phase.toUpperCase();
      equipmentProgress = combat.phase_progress;
    } else {
      equipmentState = "UNAVAILABLE";
      equipmentProgress = 0;
    }
  }
  const notice = context.combat_notice;

  return {
    home_name: context.home_name.toUpperCase(),
    away_name: context.away_name.toUpperCase(),
    home_score: scoreboard.home_score,
    away_score: scoreboard.away_score,
    clock: formatClock(scoreboard.time_left),
    venue: `${context.arena_name} · ${context.arena_location}`.toUpperCase(),
    possession: possession.toUpperCase(),
    possession_marker: ownerTeam !== undefined && ownerTeam === scoreboard.controlled_team ? "filled" : "outline",
    player_name: presentation.name.toUpperCase(),
    player_detail: `${presentation.species_name} ${presentation.position}`.toUpperCase(),
    player_state: playerState,
    species_shape: scoreboard.species_shape,
    species_color: scoreboard.species_color,
    stamina: clamp01(scoreboard.controlled_stamina),
    plan: `PLAN · ${context.tactic_name} · ${context.formation_name}`.toUpperCase(),
    ...(context.prompt !== undefined ? { prompt: context.prompt } : {}),
    ...(title !== undefined ? { announcement_title: title } : {}),
    ...(detail !== undefined ? { announcement_detail: detail } : {}),
    ...(context.phase !== undefined ? { announcement_kind: context.phase } : {}),
    ...(equipmentLabel !== undefined ? { equipment_label: equipmentLabel } : {}),
    ...(equipmentState !== undefined ? { equipment_state: equipmentState } : {}),
    equipment_progress: clamp01(equipmentProgress),
    ...(notice !== undefined ? { feedback_text: notice.text } : {}),
    ...(notice !== undefined ? { feedback_glyph: notice.glyph } : {}),
  };
}

function rectAt(ox: number, oy: number, scale: number, x: number, y: number, w: number, h: number): Rect {
  return { x: ox + x * scale, y: oy + y * scale, w: w * scale, h: h * scale };
}

function layout(viewport: { readonly w: number; readonly h: number }): MatchHudLayout {
  const scale = Math.min(viewport.w / 960, viewport.h / 540);
  const ox = (viewport.w - 960 * scale) / 2;
  const oy = (viewport.h - 540 * scale) / 2;
  const rect = (x: number, y: number, w: number, h: number): Rect => rectAt(ox, oy, scale, x, y, w, h);
  return {
    venue: rect(230, 7, 500, 14),
    scorebug: rect(230, 24, 500, 48),
    clock: rect(744, 32, 86, 32),
    status: rect(300, 76, 360, 20),
    identity: rect(24, 452, 340, 64),
    plan: rect(696, 468, 240, 44),
    prompt: rect(270, 402, 420, 52),
    announcement: rect(180, 214, 600, 104),
    combat: rect(696, 436, 240, 28),
    scale,
  };
}

export const hud = { formatClock, model, layout };
