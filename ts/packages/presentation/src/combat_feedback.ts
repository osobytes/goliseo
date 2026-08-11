// Ported from game/presentation/combat_feedback.lua.
//
// This module never reads or writes simulation state directly — every
// function here operates on an already-detached `CombatEvent` record (or on
// this module's own `CombatFeedbackState`, which is purely local HUD/camera
// bookkeeping: notices, a camera impulse, dedup ids). Nothing computed here
// can flow back into the sim. See v2/README.md §2 and the task's rollback
// note: correction smoothing is presentation, strictly one-directional.

import { combat, type ActionFamilyId, type CombatContactResult, type CombatEvent } from "./combat.ts";
import type { CombatPresentationEventId, CombatRequestRejectionReason } from "./combat.ts";

export type CombatFeedbackLinkKind = "accepted_encounter" | "rejected_request" | "missing_feedback";
export type CombatFeedbackCueState = "speculative" | "confirmed" | "revoked";

export type CombatFeedbackVfx =
  | "none"
  | "commit"
  | "projectile_spawn"
  | "projectile_expire"
  | "contact_hit"
  | "contact_extended"
  | "contact_guarded"
  | "contact_immune"
  | "contact_superseded"
  | "ball_spill"
  | "forced"
  | "guard_recoil"
  | "rejected";

export interface CombatFeedbackDisposition {
  readonly vfx: CombatFeedbackVfx;
  readonly audio?: string;
  readonly hud_text?: string;
  readonly glyph: string;
  readonly camera_strength: number;
  readonly reversible: boolean;
}

export interface CombatFeedbackLink {
  readonly stable_id: string;
  readonly link_kind: CombatFeedbackLinkKind;
  readonly semantic_id: string;
  readonly tick: number;
  readonly source_index?: number;
  readonly target_index?: number;
  readonly source_sequence?: number;
  readonly family_id?: ActionFamilyId;
  readonly result?: CombatContactResult;
  readonly reason?: CombatRequestRejectionReason;
  readonly feedback_event_id?: string;
  readonly x?: number;
  readonly y?: number;
  readonly disposition: CombatFeedbackDisposition;
}

export interface CombatFeedbackNotice {
  readonly id: string;
  readonly text: string;
  readonly glyph: string;
  readonly source_index?: number;
  readonly target_index?: number;
  life: number;
  readonly max: number;
}

export interface CombatFeedbackImpulse {
  readonly id: string;
  strength: number;
  readonly direction: number;
  life: number;
  readonly max: number;
}

export interface CombatFeedbackState {
  readonly confirmed_ids: Set<string>;
  notices: CombatFeedbackNotice[];
  impulse?: CombatFeedbackImpulse;
  reduced_motion: boolean;
  reduced_flash: boolean;
}

export interface CombatFeedbackObservation {
  readonly stable_id: string;
  readonly link_kind: CombatFeedbackLinkKind;
  readonly semantic_id: string;
  readonly cue_state: CombatFeedbackCueState;
  readonly feedback_event_id?: string;
  readonly source_sequence?: number;
  readonly reason?: CombatRequestRejectionReason;
  readonly vfx: CombatFeedbackVfx;
  readonly audio?: string;
  readonly hud_text?: string;
  readonly camera_enabled: boolean;
  readonly reduced_motion: boolean;
  readonly reduced_flash: boolean;
}

export interface CombatFeedbackDiagnostics {
  readonly confirmed_count: number;
  readonly notice_count: number;
  readonly notice_id?: string;
  readonly impulse_id?: string;
  readonly reduced_motion: boolean;
  readonly reduced_flash: boolean;
}

/** Minimal settings shape this module reads (`GameSettings` in the Lua original). */
export interface CombatFeedbackSettings {
  readonly screen_shake: boolean;
  readonly bloom: boolean;
}

const NOTICE_SECONDS = 0.72;
const MAX_NOTICE_COUNT = 6;
const IMPULSE_SECONDS = 0.16;
const MAX_CAMERA_OFFSET = 4;
let defaultReducedMotion = false;
let defaultReducedFlash = false;

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

const BASE_DISPOSITIONS: Record<
  Exclude<CombatPresentationEventId, `combat.request.rejected.${CombatRequestRejectionReason}`>,
  CombatFeedbackDisposition
> = {
  "combat.commit": {
    vfx: "commit",
    audio: "combat_commit",
    hud_text: "EQUIPMENT COMMITTED",
    glyph: "chevron",
    camera_strength: 0,
    reversible: true,
  },
  "combat.projectile.spawn": {
    vfx: "projectile_spawn",
    audio: "combat_projectile",
    glyph: "diamond",
    camera_strength: 0,
    reversible: true,
  },
  "combat.projectile.expire": {
    vfx: "projectile_expire",
    audio: "combat_expire",
    hud_text: "RANGED EXPIRED",
    glyph: "hollow_diamond",
    camera_strength: 0,
    reversible: true,
  },
  "combat.contact.hit": {
    vfx: "contact_hit",
    audio: "combat_hit",
    hud_text: "CLEAN HIT",
    glyph: "cross",
    camera_strength: 3,
    reversible: true,
  },
  "combat.contact.extended": {
    vfx: "contact_extended",
    audio: "combat_extended",
    hud_text: "CHAIN CAPPED",
    glyph: "double_cross",
    camera_strength: 2.5,
    reversible: true,
  },
  "combat.contact.guarded": {
    vfx: "contact_guarded",
    audio: "combat_guarded",
    hud_text: "GUARDED",
    glyph: "shield",
    camera_strength: 1,
    reversible: true,
  },
  "combat.contact.immune": {
    vfx: "contact_immune",
    audio: "combat_immune",
    hud_text: "IMMUNE",
    glyph: "broken_ring",
    camera_strength: 0,
    reversible: true,
  },
  "combat.contact.superseded": {
    vfx: "contact_superseded",
    audio: "combat_superseded",
    hud_text: "CONTACT DEFLECTED",
    glyph: "split",
    camera_strength: 0,
    reversible: true,
  },
  "combat.ball_spill": {
    vfx: "ball_spill",
    audio: "combat_spill",
    hud_text: "BALL SPILLED",
    glyph: "burst",
    camera_strength: 2,
    reversible: true,
  },
  "combat.forced": {
    vfx: "forced",
    audio: "combat_forced",
    hud_text: "CONTROL INTERRUPTED",
    glyph: "stagger",
    camera_strength: 3,
    reversible: true,
  },
  "combat.guard_recoil": {
    vfx: "guard_recoil",
    audio: "combat_recoil",
    hud_text: "GUARD HELD",
    glyph: "shield",
    camera_strength: 1,
    reversible: true,
  },
  // Lifecycle terminals are evidence rows: they close an accepted encounter
  // that the player already saw commit, and they add no second cue for a
  // beat the commit, contact, or recovery pose has already told.
  "combat.miss": {
    vfx: "none",
    glyph: "none",
    camera_strength: 0,
    reversible: true,
  },
  "combat.interrupted": {
    vfx: "none",
    glyph: "none",
    camera_strength: 0,
    reversible: true,
  },
  "combat.cancelled": {
    vfx: "none",
    glyph: "none",
    camera_strength: 0,
    reversible: true,
  },
  "combat.match_terminated": {
    vfx: "none",
    glyph: "none",
    camera_strength: 0,
    reversible: true,
  },
};

const REJECTION_LABELS: Readonly<Record<CombatRequestRejectionReason, string>> = {
  protected_keeper_or_no_loadout: "NO EQUIPMENT",
  kickoff_hold: "WAIT FOR KICKOFF",
  soccer_commitment: "SOCCER ACTION ACTIVE",
  aerial_state_or_recovery: "AERIAL ACTION ACTIVE",
  forced_state: "CONTROL INTERRUPTED",
  already_committed: "ACTION COMMITTED",
  cooldown: "COOLDOWN",
  missing_press_edge: "PRESS AGAIN",
  malformed_input: "INPUT INVALID",
};

function rejectedDisposition(reason: CombatRequestRejectionReason): CombatFeedbackDisposition {
  return {
    vfx: "rejected",
    audio: "combat_rejected",
    hud_text: `EQUIPMENT BLOCKED · ${REJECTION_LABELS[reason]}`,
    glyph: "blocked",
    camera_strength: 0,
    reversible: true,
  };
}

// The authoritative stream carries rejected requests too, so the same
// dispositions have to be reachable through the shared event lookup instead
// of only through `rejectedDisposition` directly.
const REJECTION_DISPOSITIONS = Object.fromEntries(
  (Object.keys(REJECTION_LABELS) as CombatRequestRejectionReason[]).map((reason) => [
    `combat.request.rejected.${reason}`,
    rejectedDisposition(reason),
  ])
) as Record<`combat.request.rejected.${CombatRequestRejectionReason}`, CombatFeedbackDisposition>;

const DISPOSITIONS: Readonly<Record<CombatPresentationEventId, CombatFeedbackDisposition>> = {
  ...BASE_DISPOSITIONS,
  ...REJECTION_DISPOSITIONS,
};

function missingDisposition(): CombatFeedbackDisposition {
  return {
    vfx: "none",
    glyph: "none",
    camera_strength: 0,
    reversible: false,
  };
}

function disposition(evt: CombatEvent): CombatFeedbackDisposition {
  const presented = combat.event(evt);
  const found = DISPOSITIONS[presented.semantic_id];
  if (found === undefined) {
    throw new Error("unsupported combat feedback event");
  }
  return found;
}

function accepted(evt: CombatEvent, stableId: string): CombatFeedbackLink {
  if (stableId === "") {
    throw new Error("combat feedback id is required");
  }
  const presented = combat.event(evt, stableId);
  if (evt.source_sequence === undefined) {
    throw new Error("accepted combat feedback requires an authoritative source sequence");
  }
  const found = DISPOSITIONS[presented.semantic_id];
  if (found === undefined) {
    throw new Error("unsupported combat feedback event");
  }
  return {
    stable_id: stableId,
    link_kind: "accepted_encounter",
    semantic_id: presented.semantic_id,
    tick: evt.tick,
    source_sequence: evt.source_sequence,
    feedback_event_id: stableId,
    x: evt.x,
    y: evt.y,
    disposition: found,
    ...definedFields<CombatFeedbackLink>({
      source_index: evt.source_index,
      target_index: evt.target_index,
      family_id: evt.family_id,
      result: evt.result,
    }),
  };
}

function rejectedRequest(
  stableId: string,
  tick: number,
  playerIndex: number,
  reason: CombatRequestRejectionReason
): CombatFeedbackLink {
  if (stableId === "") {
    throw new Error("rejected request id is required");
  }
  if (REJECTION_LABELS[reason] === undefined) {
    throw new Error("unknown combat rejection reason");
  }
  return {
    stable_id: stableId,
    link_kind: "rejected_request",
    semantic_id: `combat.request.rejected.${reason}`,
    tick,
    source_index: playerIndex,
    feedback_event_id: stableId,
    reason,
    disposition: rejectedDisposition(reason),
  };
}

// The one entry point for an authoritative combat row. An accepted encounter
// and a rejected request are different link kinds, and only the simulation
// knows which one a row is, so the caller must not have to ask.
function link(evt: CombatEvent, stableId: string): CombatFeedbackLink {
  if (evt.kind === "request_rejected") {
    if (evt.source_index === undefined) {
      throw new Error("a rejected combat request names its requesting player");
    }
    if (evt.reason === undefined) {
      throw new Error("a rejected combat request requires its typed reason");
    }
    return rejectedRequest(stableId, evt.tick, evt.source_index, evt.reason);
  }
  return accepted(evt, stableId);
}

function missingFeedback(
  requestId: string,
  tick: number,
  playerIndex: number,
  reason: CombatRequestRejectionReason
): CombatFeedbackLink {
  if (requestId === "") {
    throw new Error("missing feedback request id is required");
  }
  if (REJECTION_LABELS[reason] === undefined) {
    throw new Error("unknown combat rejection reason");
  }
  return {
    stable_id: requestId,
    link_kind: "missing_feedback",
    semantic_id: "combat.request.missing_feedback",
    tick,
    source_index: playerIndex,
    reason,
    disposition: missingDisposition(),
  };
}

function newState(): CombatFeedbackState {
  return {
    confirmed_ids: new Set(),
    notices: [],
    reduced_motion: defaultReducedMotion,
    reduced_flash: defaultReducedFlash,
  };
}

function configureDefaults(settings: CombatFeedbackSettings): void {
  defaultReducedMotion = !settings.screen_shake;
  defaultReducedFlash = !settings.bloom;
}

function configure(state: CombatFeedbackState, reducedMotion: boolean, reducedFlash: boolean): void {
  state.reduced_motion = reducedMotion;
  state.reduced_flash = reducedFlash;
  if (reducedMotion) {
    delete state.impulse;
  }
}

function stableDirection(id: string): number {
  let value = 0;
  for (let index = 0; index < id.length; index += 1) {
    // Lua strings are 1-indexed and `id:byte(index)` reads the index'th
    // byte; `index` there runs 1..#id, matching `index + 1` here.
    value = (value + id.charCodeAt(index) * (index + 1)) % 2;
  }
  return value === 0 ? -1 : 1;
}

function confirm(state: CombatFeedbackState, feedbackLink: CombatFeedbackLink): boolean {
  if (state.confirmed_ids.has(feedbackLink.stable_id)) {
    return false;
  }
  state.confirmed_ids.add(feedbackLink.stable_id);
  const feedbackDisposition = feedbackLink.disposition;
  if (feedbackDisposition.hud_text !== undefined) {
    state.notices.push({
      id: feedbackLink.stable_id,
      text: feedbackDisposition.hud_text,
      glyph: feedbackDisposition.glyph,
      life: NOTICE_SECONDS,
      max: NOTICE_SECONDS,
      ...definedFields<CombatFeedbackNotice>({
        source_index: feedbackLink.source_index,
        target_index: feedbackLink.target_index,
      }),
    });
    if (state.notices.length > MAX_NOTICE_COUNT) {
      state.notices.shift();
    }
  }
  if (feedbackDisposition.camera_strength > 0 && !state.reduced_motion) {
    const strength = Math.min(MAX_CAMERA_OFFSET, feedbackDisposition.camera_strength);
    const current = state.impulse?.strength ?? 0;
    state.impulse = {
      id: feedbackLink.stable_id,
      strength: Math.min(MAX_CAMERA_OFFSET, current + strength),
      direction: stableDirection(feedbackLink.stable_id),
      life: IMPULSE_SECONDS,
      max: IMPULSE_SECONDS,
    };
  }
  return true;
}

function resetVisuals(state: CombatFeedbackState): void {
  state.notices = [];
  delete state.impulse;
}

function tick(state: CombatFeedbackState, dt: number): void {
  for (let index = state.notices.length - 1; index >= 0; index -= 1) {
    const notice = state.notices[index];
    if (notice === undefined) {
      continue;
    }
    notice.life = Math.max(0, notice.life - dt);
    if (notice.life === 0) {
      state.notices.splice(index, 1);
    }
  }
  if (state.impulse !== undefined) {
    state.impulse.life = Math.max(0, state.impulse.life - dt);
    if (state.impulse.life === 0) {
      delete state.impulse;
    }
  }
}

function cameraOffset(state: CombatFeedbackState): { x: number; y: number } {
  if (state.reduced_motion || state.impulse === undefined) {
    return { x: 0, y: 0 };
  }
  const impulse = state.impulse;
  const progress = impulse.life / impulse.max;
  const magnitude = Math.min(MAX_CAMERA_OFFSET, impulse.strength * progress);
  return {
    x: impulse.direction * magnitude,
    y: -magnitude * 0.35,
  };
}

function notice(state: CombatFeedbackState, controlledIndex: number): CombatFeedbackNotice | null {
  for (let index = state.notices.length - 1; index >= 0; index -= 1) {
    const candidate = state.notices[index];
    if (
      candidate !== undefined &&
      (candidate.source_index === controlledIndex || candidate.target_index === controlledIndex)
    ) {
      return candidate;
    }
  }
  return null;
}

function observation(
  state: CombatFeedbackState,
  feedbackLink: CombatFeedbackLink,
  cueState: CombatFeedbackCueState
): CombatFeedbackObservation {
  return {
    stable_id: feedbackLink.stable_id,
    link_kind: feedbackLink.link_kind,
    semantic_id: feedbackLink.semantic_id,
    cue_state: cueState,
    vfx: feedbackLink.disposition.vfx,
    camera_enabled: feedbackLink.disposition.camera_strength > 0 && !state.reduced_motion,
    reduced_motion: state.reduced_motion,
    reduced_flash: state.reduced_flash,
    ...definedFields<CombatFeedbackObservation>({
      feedback_event_id: feedbackLink.feedback_event_id,
      source_sequence: feedbackLink.source_sequence,
      reason: feedbackLink.reason,
      audio: feedbackLink.disposition.audio,
      hud_text: feedbackLink.disposition.hud_text,
    }),
  };
}

function diagnostics(state: CombatFeedbackState): CombatFeedbackDiagnostics {
  const lastNotice = state.notices[state.notices.length - 1];
  return {
    confirmed_count: state.confirmed_ids.size,
    notice_count: state.notices.length,
    reduced_motion: state.reduced_motion,
    reduced_flash: state.reduced_flash,
    ...definedFields<CombatFeedbackDiagnostics>({
      notice_id: lastNotice?.id,
      impulse_id: state.impulse?.id,
    }),
  };
}

export const combatFeedback = {
  disposition,
  accepted,
  rejectedRequest,
  link,
  missingFeedback,
  new: newState,
  configureDefaults,
  configure,
  confirm,
  resetVisuals,
  tick,
  cameraOffset,
  notice,
  observation,
  diagnostics,
};
