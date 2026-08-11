import { describe, expect, it } from "vitest";
import { CompatibilityMetrics } from "./compatibility_metrics.ts";

describe("compatibility metrics", () => {
  it("excludes warm-up work and measures the next update after input", () => {
    const metrics = CompatibilityMetrics.new(0);

    metrics.beginUpdate(0);
    metrics.finishUpdate(0.001);
    metrics.beginUpdate(10);
    metrics.finishUpdate(10.001);

    expect(metrics.sampleNumber).toBe(1);
    expect(metrics.updateSamples.length).toBe(1);
    expect(metrics.frameSamples.length).toBe(0);

    metrics.input(10.1, "key_return");
    metrics.beginUpdate(10.1);
    metrics.finishUpdate(10.103);
    metrics.beginDraw(10.103);
    metrics.finishDraw(10.105);

    expect(metrics.inputSamples.length).toBe(1);
    expect(metrics.inputSamples[0]).toBeCloseTo(3, 9);
    expect(metrics.drawSamples.length).toBe(1);
  });

  it("records browser-observable runtime state", () => {
    const metrics = CompatibilityMetrics.new(0);
    const value = {
      master_volume: 1,
      sfx_volume: 0.8,
      crowd_volume: 0.55,
      muted: true,
      fullscreen: false,
      screen_shake: true,
      bloom: true,
    };

    metrics.settings(0.5, value);
    metrics.audio(0.6, 1, 1);

    expect(metrics.sampleNumber).toBe(0);
  });

  it("starts a new bounded sample after sixty seconds", () => {
    const metrics = CompatibilityMetrics.new(0);
    metrics.beginUpdate(10);
    metrics.finishUpdate(10.001);
    metrics.beginUpdate(70.1);
    metrics.finishUpdate(70.101);

    expect(metrics.sampleNumber).toBe(2);
    expect(metrics.updateSamples.length).toBe(1);
    expect(metrics.frameSamples.length).toBe(0);
  });
});
