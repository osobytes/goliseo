// Synthesized audio for the match screen. All SFX are generated at load
// time from mathematical waveforms -- no asset files. Each SFX is <= 0.4s at
// 22050 Hz mono. Offline playback keeps the state-event adapter; rollback
// playback consumes confirmed stable events. The crowd bed is a continuous
// low loop.
//
// HEADLESS CONTRACT: there is no browser audio implementation in this
// milestone -- wiring a real backend is a later
// milestone's job, "do not build it, and do not block on it". Every method
// on `Audio` must no-op cleanly when constructed without a backend.
// `AudioBackend` is this module's `GraphicsBackend` equivalent
// (`@gc/ui/src/graphics_backend.ts`): nothing in this package implements it.
//
// The low-level sample builders below are pure functions writing into a
// plain `Float32Array` -- pure synthesis math with no backend dependency at
// all -- so `AudioBackend` only needs to own actual playback
// (creating/cloning/playing/stopping a source), mirroring how
// `graphics_backend.ts` abstracts drawing while `draw.ts`'s layout math
// stays pure.
//
// `@gc/presentation`'s `combatFeedback` module owns `link`/`disposition`,
// which is not a declared dependency of this package, so they are injected
// as a `CombatFeedbackPort`.

export interface AudioSourceHandle {
  readonly id: number;
}

/**
 * The presentation-agnostic audio playback surface `Audio` renders SFX
 * through. Wiring a real implementation (Web Audio or similar) is out of
 * scope for this milestone -- see this file's header.
 */
export interface AudioBackend {
  /** Creates a static, playable audio source from raw samples. */
  newSource(samples: Float32Array, sampleRate: number): AudioSourceHandle;
  /** Clones a source, so an overlapping one-shot hit can play without interrupting the original. */
  clone(handle: AudioSourceHandle): AudioSourceHandle;
  play(handle: AudioSourceHandle): void;
  stop(handle: AudioSourceHandle): void;
  isPlaying(handle: AudioSourceHandle): boolean;
  setVolume(handle: AudioSourceHandle, volume: number): void;
  setLooping(handle: AudioSourceHandle, looping: boolean): void;
}

/** `@gc/presentation`'s `combatFeedback.link`/`.disposition`, injected -- see this file's header. */
export interface CombatFeedbackPort {
  link(payload: unknown, stableId: string): { readonly disposition: { readonly audio?: string } };
  disposition(event: unknown): { readonly audio?: string };
}

export interface GameSettingsAudioSlice {
  readonly sfx_volume: number;
  readonly crowd_volume: number;
  readonly muted: boolean;
}

/** `gc-sim`'s `rollback_events.rs`'s `RollbackWrappedEvent` (Rust-owned; only the fields this module reads). */
export interface RollbackWrappedEvent {
  readonly id: string;
  readonly domain: string;
  readonly payload: { readonly kind?: string } & Record<string, unknown>;
}

export interface CombatEventLike {
  readonly kind?: string;
}

/** The slice of `gc-sim`'s `match.rs` `MatchState` `audio.consume` reads. */
export interface AudioMatchState {
  readonly events: readonly { readonly kind: string }[];
  readonly score: { readonly home: number; readonly away: number };
  readonly finished: boolean;
  readonly time_left: number;
}

const RATE = 22050;

const BASE_SFX_VOLUME = 0.5;
const BASE_CROWD_VOLUME = 0.08;
const BASE_GOAL_SWELL_VOLUME = 0.22;
const CROWD_SWELL_DUR = 3.0;

const CUE_GAIN: Readonly<Record<string, number>> = {
  touch: 0.45,
  reception: 0.5,
  pass: 0.62,
  block: 0.68,
  catch: 0.7,
  claim: 0.7,
  parry: 0.76,
  tackle: 0.84,
  header: 0.86,
  shot: 1.0,
  volley: 1.0,
  bicycle: 1.0,
  kickoff: 0.82,
  goal: 1.15,
  full_time: 1.12,
  combat_commit: 0.5,
  combat_projectile: 0.62,
  combat_expire: 0.42,
  combat_hit: 0.9,
  combat_extended: 0.82,
  combat_guarded: 0.78,
  combat_immune: 0.54,
  combat_superseded: 0.48,
  combat_spill: 0.84,
  combat_forced: 0.88,
  combat_recoil: 0.7,
  combat_rejected: 0.42,
};

const AUDIBLE_MATCH_KINDS: ReadonlySet<string> = new Set([
  "shot",
  "pass",
  "touch",
  "tackle",
  "block",
  "catch",
  "claim",
  "parry",
  "header",
  "volley",
  "bicycle",
  "reception",
  "first_touch_shot",
]);

// --- pure sample synthesis --------------------------------------------------

function clampSample(value: number): number {
  return Math.max(-1, Math.min(1, value));
}

function writeSineDecay(samples: Float32Array, freq: number, decay: number, amp: number): void {
  const n = samples.length;
  for (let i = 0; i < n; i += 1) {
    const t = i / RATE;
    samples[i] = clampSample(Math.sin(2 * Math.PI * freq * t) * Math.exp(-decay * t) * amp);
  }
}

function writeTriDecay(samples: Float32Array, freq: number, decay: number, amp: number): void {
  const n = samples.length;
  const period = RATE / freq;
  for (let i = 0; i < n; i += 1) {
    const t = i / RATE;
    const phase = (i % period) / period;
    const tri = 2 * Math.abs(2 * phase - 1) - 1;
    samples[i] = clampSample(tri * Math.exp(-decay * t) * amp);
  }
}

function writeNoiseDecay(samples: Float32Array, decay: number, amp: number): void {
  const n = samples.length;
  let seed = 12345;
  for (let i = 0; i < n; i += 1) {
    const t = i / RATE;
    seed = (seed * 1664525 + 1013904223) % 2 ** 32;
    const r = seed / 2 ** 31 - 1;
    samples[i] = clampSample(r * Math.exp(-decay * t) * amp);
  }
}

function mixSine(samples: Float32Array, freq: number, decay: number, amp: number): void {
  const n = samples.length;
  for (let i = 0; i < n; i += 1) {
    const t = i / RATE;
    const add = Math.sin(2 * Math.PI * freq * t) * Math.exp(-decay * t) * amp;
    const cur = samples[i] ?? 0;
    samples[i] = clampSample(cur + add);
  }
}

function makeSamples(durSeconds: number, fill: (samples: Float32Array) => void): Float32Array {
  const samples = new Float32Array(Math.floor(Math.min(durSeconds, 0.4) * RATE));
  fill(samples);
  return samples;
}

function buildPass(): Float32Array {
  return makeSamples(0.12, (sd) => writeSineDecay(sd, 880, 30, 0.7));
}

function buildTouch(): Float32Array {
  return makeSamples(0.1, (sd) => {
    writeSineDecay(sd, 660, 28, 0.6);
    mixSine(sd, 330, 40, 0.15);
  });
}

function buildShot(): Float32Array {
  return makeSamples(0.25, (sd) => {
    writeNoiseDecay(sd, 18, 0.55);
    mixSine(sd, 90, 14, 0.45);
  });
}

function buildTackle(): Float32Array {
  return makeSamples(0.18, (sd) => {
    writeNoiseDecay(sd, 22, 0.6);
    mixSine(sd, 120, 20, 0.35);
  });
}

function buildBlock(): Float32Array {
  return makeSamples(0.15, (sd) => {
    writeNoiseDecay(sd, 25, 0.7);
    mixSine(sd, 200, 22, 0.3);
  });
}

function buildCatch(): Float32Array {
  return makeSamples(0.14, (sd) => {
    writeNoiseDecay(sd, 30, 0.8);
    mixSine(sd, 1200, 35, 0.3);
  });
}

function buildParry(): Float32Array {
  return makeSamples(0.18, (sd) => {
    writeSineDecay(sd, 1400, 28, 0.5);
    writeNoiseDecay(sd, 35, 0.35);
  });
}

function buildHeader(): Float32Array {
  return makeSamples(0.15, (sd) => writeTriDecay(sd, 420, 24, 0.55));
}

function buildVolley(): Float32Array {
  return makeSamples(0.3, (sd) => {
    writeNoiseDecay(sd, 14, 0.65);
    mixSine(sd, 70, 10, 0.5);
  });
}

function buildGoal(): Float32Array {
  return makeSamples(0.4, (sd) => {
    const n = sd.length;
    let seed = 99991;
    for (let i = 0; i < n; i += 1) {
      const t = i / RATE;
      const frac = t / 0.4;
      const freq = 400 + 400 * frac;
      const tone1 = Math.sin(2 * Math.PI * freq * t) * 0.4;
      const freq2 = freq * 1.2;
      const tone2 = Math.sin(2 * Math.PI * freq2 * t) * 0.3;
      seed = (seed * 1664525 + 1013904223) % 2 ** 32;
      const r = seed / 2 ** 31 - 1;
      const noise = r * frac * 0.25;
      const env = Math.exp(-3 * (1 - frac));
      sd[i] = clampSample((tone1 + tone2 + noise) * env);
    }
  });
}

function buildKickoff(): Float32Array {
  return makeSamples(0.35, (sd) => {
    const n = sd.length;
    const period1End = Math.floor(0.12 * RATE);
    const gapEnd = Math.floor(0.2 * RATE);
    const period = RATE / 880;
    for (let i = 0; i < n; i += 1) {
      let s = 0.0;
      if (i < period1End || i >= gapEnd) {
        let decay: number;
        if (i < period1End) {
          decay = i / period1End;
        } else {
          decay = (n - 1 - i) / (n - 1 - gapEnd);
        }
        const phase = (i % period) / period;
        const tri = 2 * Math.abs(2 * phase - 1) - 1;
        s = tri * decay * 0.65;
      }
      sd[i] = clampSample(s);
    }
  });
}

function buildFullTime(): Float32Array {
  return makeSamples(0.4, (sd) => {
    const n = sd.length;
    const period = RATE / 1040;
    for (let i = 0; i < n; i += 1) {
      const t = i / RATE;
      let whistle = 0;
      if (t < 0.13 || (t >= 0.2 && t < 0.38)) {
        const localT = t < 0.13 ? t : t - 0.2;
        const env = Math.max(0, 1 - localT / 0.18);
        const phase = (i % period) / period;
        whistle = (2 * Math.abs(2 * phase - 1) - 1) * env * 0.55;
      }
      const closing = Math.sin(2 * Math.PI * 130 * t) * Math.exp(-5 * t) * 0.2;
      sd[i] = clampSample(whistle + closing);
    }
  });
}

function buildCrowd(): Float32Array {
  const samples = new Float32Array(2 * RATE);
  let seed = 55555;
  let prev = 0.0;
  const alpha = 0.06;
  for (let i = 0; i < samples.length; i += 1) {
    seed = (seed * 1664525 + 1013904223) % 2 ** 32;
    const r = seed / 2 ** 31 - 1;
    prev = prev + alpha * (r - prev);
    samples[i] = clampSample(prev * 2.5);
  }
  return samples;
}

// --- public API --------------------------------------------------------------

/** Mutable per-match/per-session audio state -- one instance per screen. */
export class Audio {
  private readonly backend: AudioBackend | undefined;
  private readonly combatFeedback: CombatFeedbackPort;

  private muted = false;
  private loaded = false;
  private sfxMix = 1;
  private crowdMix = 1;

  private crowdSource: AudioSourceHandle | undefined;
  private readonly sfx = new Map<string, AudioSourceHandle>();

  private prevScoreHome = 0;
  private prevScoreAway = 0;
  private prevFinished = false;
  private prevTimeLeft = -1;
  private crowdSwellT = 0.0;
  private readonly confirmedIds = new Set<string>();
  private confirmedCues = new Map<string, number>();
  private suppressedConfirmedCues = new Map<string, number>();

  constructor(combatFeedback: CombatFeedbackPort, backend?: AudioBackend) {
    this.combatFeedback = combatFeedback;
    this.backend = backend;
  }

  private sfxVolume(): number {
    return BASE_SFX_VOLUME * this.sfxMix;
  }

  private crowdVolume(): number {
    return BASE_CROWD_VOLUME * this.crowdMix;
  }

  private goalSwellVolume(): number {
    return Math.min(1, BASE_GOAL_SWELL_VOLUME * this.crowdMix);
  }

  /** Synthesize all SFX. Must be called once before update/reset. No-ops without a backend. */
  load(): void {
    if (!this.backend || this.loaded) {
      return;
    }
    this.loaded = true;
    const backend = this.backend;
    const source = (samples: Float32Array): AudioSourceHandle => {
      const handle = backend.newSource(samples, RATE);
      backend.setVolume(handle, this.sfxVolume());
      return handle;
    };

    this.sfx.set("pass", source(buildPass()));
    this.sfx.set("touch", source(buildTouch()));
    this.sfx.set("shot", source(buildShot()));
    this.sfx.set("tackle", source(buildTackle()));
    this.sfx.set("block", source(buildBlock()));
    this.sfx.set("catch", source(buildCatch()));
    this.sfx.set("claim", source(buildCatch()));
    this.sfx.set("parry", source(buildParry()));
    this.sfx.set("header", source(buildHeader()));
    this.sfx.set("volley", source(buildVolley()));
    this.sfx.set("first_touch_shot", source(buildVolley()));
    this.sfx.set("bicycle", source(buildVolley()));
    this.sfx.set("reception", source(buildPass()));
    this.sfx.set("goal", source(buildGoal()));
    this.sfx.set("kickoff", source(buildKickoff()));
    this.sfx.set("full_time", source(buildFullTime()));
    this.sfx.set("combat_commit", source(buildPass()));
    this.sfx.set("combat_projectile", source(buildParry()));
    this.sfx.set("combat_expire", source(buildTouch()));
    this.sfx.set("combat_hit", source(buildTackle()));
    this.sfx.set("combat_extended", source(buildTackle()));
    this.sfx.set("combat_guarded", source(buildBlock()));
    this.sfx.set("combat_immune", source(buildParry()));
    this.sfx.set("combat_superseded", source(buildPass()));
    this.sfx.set("combat_spill", source(buildCatch()));
    this.sfx.set("combat_forced", source(buildVolley()));
    this.sfx.set("combat_recoil", source(buildBlock()));
    this.sfx.set("combat_rejected", source(buildTouch()));

    const crowd = backend.newSource(buildCrowd(), RATE);
    backend.setLooping(crowd, true);
    backend.setVolume(crowd, this.crowdVolume());
    this.crowdSource = crowd;
    if (!this.muted) {
      backend.play(crowd);
    }
  }

  /** Play a named SFX by cloning the static source. No-ops when muted or backendless. */
  private play(name: string): void {
    if (this.muted || !this.backend) {
      return;
    }
    const src = this.sfx.get(name);
    if (!src) {
      return;
    }
    const clone = this.backend.clone(src);
    this.backend.setVolume(clone, Math.min(1, this.sfxVolume() * (CUE_GAIN[name] ?? 1)));
    this.backend.play(clone);
  }

  private countConfirmedCue(name: string): void {
    this.confirmedCues.set(name, (this.confirmedCues.get(name) ?? 0) + 1);
  }

  private startGoalSwell(): void {
    this.crowdSwellT = CROWD_SWELL_DUR;
    if (this.crowdSource && !this.muted && this.backend) {
      this.backend.setVolume(this.crowdSource, this.goalSwellVolume());
    }
  }

  // Consume one stable event only after its rollback step is confirmed. Event
  // identity, rather than mutable score/time edges, is the deduplication key.
  consumeConfirmed(event: RollbackWrappedEvent, suppressPlayback?: boolean): boolean {
    if (typeof event !== "object" || event === null || typeof event.id !== "string") {
      throw new Error("confirmed audio event is invalid");
    }
    if (this.confirmedIds.has(event.id)) {
      return false;
    }
    this.confirmedIds.add(event.id);

    let name: string | undefined;
    if (event.domain.startsWith("match/")) {
      const kind = event.payload.kind;
      if (kind !== undefined && CUE_GAIN[kind] !== undefined) {
        name = kind;
      }
    } else if (event.domain.startsWith("combat/")) {
      const link = this.combatFeedback.link(event.payload, event.id);
      name = link.disposition.audio;
    } else if (event.domain === "lifecycle/goal") {
      name = "goal";
    } else if (event.domain === "lifecycle/kickoff") {
      name = "kickoff";
    } else if (event.domain === "lifecycle/full_time") {
      name = "full_time";
    }
    if (name === undefined) {
      return true;
    }

    if (suppressPlayback) {
      this.suppressedConfirmedCues.set(name, (this.suppressedConfirmedCues.get(name) ?? 0) + 1);
      return true;
    }
    this.countConfirmedCue(name);
    this.play(name);
    if (name === "goal") {
      this.startGoalSwell();
    }
    return true;
  }

  consumeCombat(events: readonly CombatEventLike[]): void {
    for (const event of events) {
      const name = this.combatFeedback.disposition(event).audio;
      if (name !== undefined) {
        this.play(name);
      }
    }
  }

  confirmedCueCounts(): Readonly<Record<string, number>> {
    return Object.fromEntries(this.confirmedCues);
  }

  suppressedConfirmedCueCounts(): Readonly<Record<string, number>> {
    return Object.fromEntries(this.suppressedConfirmedCues);
  }

  /** Advance continuous ambience without consuming or replaying match events. */
  tick(dt: number): void {
    if (!this.backend) {
      return;
    }
    if (this.crowdSwellT > 0) {
      this.crowdSwellT -= dt;
      if (this.crowdSwellT <= 0) {
        this.crowdSwellT = 0;
        if (this.crowdSource && !this.muted) {
          this.backend.setVolume(this.crowdSource, this.crowdVolume());
        }
      }
    }
    if (this.crowdSource && !this.muted && !this.backend.isPlaying(this.crowdSource)) {
      this.backend.play(this.crowdSource);
    }
  }

  // Consume one simulated state's events and score edges. Continuous ambience
  // is deliberately advanced separately by `tick` at the display's cadence.
  consume(state: AudioMatchState): void {
    if (!this.backend) {
      return;
    }

    for (const evt of state.events) {
      if (AUDIBLE_MATCH_KINDS.has(evt.kind)) {
        this.play(evt.kind);
      }
    }

    let scored = false;
    if (state.score.home !== this.prevScoreHome || state.score.away !== this.prevScoreAway) {
      scored = true;
    }
    this.prevScoreHome = state.score.home;
    this.prevScoreAway = state.score.away;

    if (scored) {
      this.play("goal");
      this.startGoalSwell();
    }

    if (state.finished && !this.prevFinished) {
      this.play("full_time");
    }
    this.prevFinished = state.finished;

    if (this.prevTimeLeft >= 0 && state.time_left > this.prevTimeLeft + 1) {
      this.play("kickoff");
    }
    this.prevTimeLeft = state.time_left;
  }

  // Compatibility helper for callers that have one simulation step per render
  // frame. Fixed-clock callers consume every tick then advance ambience once.
  update(state: AudioMatchState, dt: number): void {
    this.consume(state);
    this.tick(dt);
  }

  /** Reset audio state for a new match (stop crowd swell, re-sync score refs). Plays the kickoff whistle. */
  reset(): void {
    this.prevScoreHome = 0;
    this.prevScoreAway = 0;
    this.crowdSwellT = 0;
    this.prevTimeLeft = -1;
    this.prevFinished = false;
    this.confirmedIds.clear();
    this.confirmedCues = new Map();
    this.suppressedConfirmedCues = new Map();

    if (!this.backend) {
      return;
    }

    if (this.crowdSource) {
      this.backend.setVolume(this.crowdSource, this.muted ? 0 : this.crowdVolume());
      if (!this.muted && !this.backend.isPlaying(this.crowdSource)) {
        this.backend.play(this.crowdSource);
      }
    }

    this.play("kickoff");
  }

  setMuted(value: boolean): boolean {
    this.muted = value;
    if (!this.backend) {
      return this.muted;
    }
    if (this.crowdSource) {
      if (this.muted) {
        this.backend.setVolume(this.crowdSource, 0);
      } else {
        this.backend.setVolume(
          this.crowdSource,
          this.crowdSwellT > 0 ? this.goalSwellVolume() : this.crowdVolume(),
        );
        if (!this.backend.isPlaying(this.crowdSource)) {
          this.backend.play(this.crowdSource);
        }
      }
    }
    return this.muted;
  }

  /** Toggle mute. Returns the new muted state. */
  toggleMute(): boolean {
    return this.setMuted(!this.muted);
  }

  configure(settings: GameSettingsAudioSlice): void {
    this.sfxMix = settings.sfx_volume / 0.8;
    this.crowdMix = settings.crowd_volume / 0.55;
    if (this.backend) {
      for (const source of this.sfx.values()) {
        this.backend.setVolume(source, this.sfxVolume());
      }
    }
    this.setMuted(settings.muted);
  }
}
