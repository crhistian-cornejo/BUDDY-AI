import { describe, expect, it } from "vitest";
import { argbToRgba } from "./sprite";

describe("argbToRgba", () => {
  it("keeps colors and transparency", () => {
    expect([...argbToRgba([0xffff0000, 0, 0xff14161a])]).toEqual([255, 0, 0, 255, 0, 0, 0, 0, 0x14, 0x16, 0x1a, 255]);
  });
});
