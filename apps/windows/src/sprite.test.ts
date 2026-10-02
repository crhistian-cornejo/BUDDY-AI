import { describe, expect, it } from "vitest";
import { argbToRgba, breathFrames } from "./sprite";

const motion = { breathEveryMin: 6, breathEveryMax: 10, breathDuration: 2, maxFps: 30 };

describe("argbToRgba", () => {
  it("keeps colors and transparency", () => {
    expect([...argbToRgba([0xffff0000, 0, 0xff14161a])]).toEqual([255, 0, 0, 255, 0, 0, 0, 0, 0x14, 0x16, 0x1a, 255]);
  });
});

describe("breathFrames", () => {
  it("breathes for the duration at the state's fps and ends at rest", () => {
    expect(breathFrames(2, 2, motion)).toEqual([1, 0, 1, 0, 0]);
  });
  it("caps the frame rate", () => {
    expect(breathFrames(2, 120, motion).length).toBe(61);
  });
  it("does nothing with a single frame", () => {
    expect(breathFrames(1, 2, motion)).toEqual([]);
  });
});
