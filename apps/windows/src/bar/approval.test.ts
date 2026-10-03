import { describe, expect, it } from "vitest";
import { seenWhole } from "./approval";

describe("seenWhole", () => {
  it("is true for a command that fits in its box", () => {
    expect(seenWhole({ scrollTop: 0, clientHeight: 96, scrollHeight: 96 })).toBe(true);
    expect(seenWhole({ scrollTop: 0, clientHeight: 96, scrollHeight: 60 })).toBe(true);
  });

  it("is false while part of the command is below the box", () => {
    expect(seenWhole({ scrollTop: 0, clientHeight: 96, scrollHeight: 400 })).toBe(false);
    expect(seenWhole({ scrollTop: 200, clientHeight: 96, scrollHeight: 400 })).toBe(false);
  });

  it("is true once the user scrolled to the end", () => {
    expect(seenWhole({ scrollTop: 304, clientHeight: 96, scrollHeight: 400 })).toBe(true);
    // Fractional scroll positions stop a pixel short.
    expect(seenWhole({ scrollTop: 302.5, clientHeight: 96, scrollHeight: 400 })).toBe(true);
  });

  it("is false for a box that has no size yet", () => {
    expect(seenWhole({ scrollTop: 0, clientHeight: 0, scrollHeight: 0 })).toBe(false);
  });
});
