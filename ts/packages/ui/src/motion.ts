// Small, pure easing helpers for screen transitions.

const REVEAL_DURATION = 0.18;

function advance(progress: number, dt: number): number {
  if (dt <= 0) {
    return progress;
  }
  return Math.min(1, progress + dt / REVEAL_DURATION);
}

/** @returns `[x, remainingWidth]`. */
function wipe(progress: number, width: number): readonly [x: number, remainingWidth: number] {
  const clamped = Math.max(0, Math.min(1, progress));
  const eased = 1 - (1 - clamped) * (1 - clamped);
  const x = width * eased;
  return [x, width - x];
}

export const motion = { advance, wipe };
