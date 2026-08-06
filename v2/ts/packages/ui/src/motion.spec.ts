import { describe, expect, it } from "vitest";
import { motion } from "./motion.ts";

describe("UI route motion", () => {
  it("finishes quickly and never exceeds its normalized range", () => {
    let progress = 0;
    for (let i = 0; i < 12; i++) {
      progress = motion.advance(progress, 1 / 60);
    }
    expect(progress).toBe(1);
    expect(motion.advance(progress, 1)).toBe(1);
    expect(motion.advance(0.5, -1)).toBe(0.5);
  });

  it("reveals the full canvas from left to right", () => {
    const [x0, width0] = motion.wipe(0, 960);
    const [x1, width1] = motion.wipe(1, 960);
    expect(x0).toBe(0);
    expect(width0).toBe(960);
    expect(x1).toBe(960);
    expect(width1).toBe(0);
  });
});
