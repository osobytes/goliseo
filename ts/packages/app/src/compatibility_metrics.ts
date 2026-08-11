// Lightweight runtime measurements shared by native and browser runs.
// Samples are emitted as pipe-delimited console lines so a browser console
// export and a native stdout capture can be compared without a transport
// bridge or a browser-specific dependency in the simulation layers.

import type { GameSettings } from "./settings.ts";

const WARMUP_SECONDS = 10;
const SAMPLE_SECONDS = 60;

export interface CompatibilityPendingInput {
  readonly at: number;
  readonly kind: string;
  readonly sequence: number;
}

type FieldValue = string | number | boolean;

function emit(kind: string, fields: Readonly<Record<string, FieldValue>>): void {
  const parts = ["GC_METRICS", kind];
  const keys = Object.keys(fields).sort();
  for (const key of keys) {
    parts.push(`${key}=${String(fields[key])}`);
  }
  // eslint-disable-next-line no-console
  console.log(parts.join("|"));
}

function sortedCopy(values: readonly number[]): number[] {
  return [...values].sort((a, b) => a - b);
}

function percentile(values: readonly number[], fraction: number): number | undefined {
  if (values.length === 0) {
    return undefined;
  }
  const sorted = sortedCopy(values);
  const index = Math.max(1, Math.ceil(sorted.length * fraction)) - 1;
  return sorted[index];
}

function maximum(values: readonly number[]): number | undefined {
  let result: number | undefined;
  for (const value of values) {
    if (result === undefined || value > result) {
      result = value;
    }
  }
  return result;
}

function metricValue(value: number | undefined): string {
  return value !== undefined ? value.toFixed(3) : "na";
}

/** The mutable measurement window this module accumulates. See file header. */
export class CompatibilityMetrics {
  readonly startedAt: number;
  readonly warmupSeconds = WARMUP_SECONDS;
  readonly sampleSeconds = SAMPLE_SECONDS;
  sampleStartedAt: number | undefined;
  sampleNumber = 0;
  updateSamples: number[] = [];
  drawSamples: number[] = [];
  frameSamples: number[] = [];
  inputSamples: number[] = [];
  pendingInputs: CompatibilityPendingInput[] = [];
  lastFrameAt: number | undefined;
  updateStartedAt: number | undefined;
  drawStartedAt: number | undefined;
  inputSequence = 0;
  flowFinished = false;

  private constructor(now: number) {
    this.startedAt = now;
  }

  static new(now: number): CompatibilityMetrics {
    const self = new CompatibilityMetrics(now);
    emit("boot", { at_ms: 0 });
    return self;
  }

  private clearSample(): void {
    this.updateSamples = [];
    this.drawSamples = [];
    this.frameSamples = [];
    this.inputSamples = [];
  }

  private emitSample(now: number, partial: boolean): void {
    const startedAt = this.sampleStartedAt;
    if (startedAt === undefined) {
      throw new Error("compatibility metrics sample has no start time");
    }
    let frameOver33 = 0;
    let frameOver250 = 0;
    for (const interval of this.frameSamples) {
      if (interval > 33) {
        frameOver33 += 1;
      }
      if (interval > 250) {
        frameOver250 += 1;
      }
    }
    emit("sample", {
      draw_max_ms: metricValue(maximum(this.drawSamples)),
      draw_p50_ms: metricValue(percentile(this.drawSamples, 0.5)),
      draw_p95_ms: metricValue(percentile(this.drawSamples, 0.95)),
      duration_s: (now - startedAt).toFixed(3),
      frame_over_250_ms: frameOver250,
      frame_over_33_ms: frameOver33,
      frame_p50_ms: metricValue(percentile(this.frameSamples, 0.5)),
      frame_p95_ms: metricValue(percentile(this.frameSamples, 0.95)),
      frames: this.frameSamples.length,
      input_max_ms: metricValue(maximum(this.inputSamples)),
      input_p50_ms: metricValue(percentile(this.inputSamples, 0.5)),
      input_p95_ms: metricValue(percentile(this.inputSamples, 0.95)),
      partial,
      sample: this.sampleNumber,
      update_max_ms: metricValue(maximum(this.updateSamples)),
      update_p50_ms: metricValue(percentile(this.updateSamples, 0.5)),
      update_p95_ms: metricValue(percentile(this.updateSamples, 0.95)),
      updates: this.updateSamples.length,
    });
  }

  private advanceSample(now: number): void {
    if (this.sampleStartedAt === undefined) {
      if (now - this.startedAt < this.warmupSeconds) {
        return;
      }
      this.sampleStartedAt = now;
      this.sampleNumber = 1;
      emit("sample_start", { sample: this.sampleNumber });
      return;
    }

    if (now - this.sampleStartedAt >= this.sampleSeconds) {
      this.emitSample(now, false);
      this.sampleNumber += 1;
      this.sampleStartedAt = now;
      this.clearSample();
      emit("sample_start", { sample: this.sampleNumber });
    }
  }

  beginUpdate(now: number): void {
    this.advanceSample(now);
    if (
      this.lastFrameAt !== undefined &&
      this.sampleStartedAt !== undefined &&
      this.lastFrameAt >= this.sampleStartedAt
    ) {
      this.frameSamples.push((now - this.lastFrameAt) * 1000);
    }
    this.lastFrameAt = now;
    this.updateStartedAt = now;
  }

  finishUpdate(now: number): void {
    this.advanceSample(now);
    if (
      this.updateStartedAt !== undefined &&
      this.sampleStartedAt !== undefined &&
      this.updateStartedAt >= this.sampleStartedAt
    ) {
      this.updateSamples.push((now - this.updateStartedAt) * 1000);
    }
    for (const input of this.pendingInputs) {
      const latency = (now - input.at) * 1000;
      emit("input_latency", {
        kind: input.kind,
        latency_ms: latency.toFixed(3),
        sequence: input.sequence,
      });
      if (this.sampleStartedAt !== undefined && input.at >= this.sampleStartedAt) {
        this.inputSamples.push(latency);
      }
    }
    this.pendingInputs = [];
  }

  beginDraw(now: number): void {
    this.advanceSample(now);
    this.drawStartedAt = now;
  }

  finishDraw(now: number): void {
    this.advanceSample(now);
    if (
      this.drawStartedAt !== undefined &&
      this.sampleStartedAt !== undefined &&
      this.drawStartedAt >= this.sampleStartedAt
    ) {
      this.drawSamples.push((now - this.drawStartedAt) * 1000);
    }
  }

  input(now: number, kind: string): void {
    this.inputSequence += 1;
    this.pendingInputs.push({ at: now, kind, sequence: this.inputSequence });
    emit("input", {
      at_ms: (now - this.startedAt) * 1000,
      kind,
      sequence: this.inputSequence,
    });
  }

  route(now: number, route: string): void {
    emit("route", { at_ms: (now - this.startedAt) * 1000, route });
  }

  lifecycle(now: number, event: string): void {
    emit("lifecycle", { at_ms: (now - this.startedAt) * 1000, event });
  }

  settings(now: number, settings: GameSettings): void {
    emit("settings", {
      at_ms: (now - this.startedAt) * 1000,
      fullscreen: settings.fullscreen,
      muted: settings.muted,
    });
  }

  audio(now: number, activeSources: number, volume: number): void {
    emit("audio", {
      active_sources: activeSources,
      at_ms: (now - this.startedAt) * 1000,
      volume,
    });
  }

  flowComplete(now: number, route: string): void {
    if (this.flowFinished) {
      return;
    }
    this.flowFinished = true;
    emit("flow_complete", { at_ms: (now - this.startedAt) * 1000, route });
  }

  finish(now: number): void {
    this.advanceSample(now);
    if (this.sampleStartedAt !== undefined && this.frameSamples.length > 0) {
      this.emitSample(now, true);
      this.clearSample();
    }
  }
}

export const compatibilityMetrics = { new: CompatibilityMetrics.new };
