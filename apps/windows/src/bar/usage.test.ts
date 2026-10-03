import { expect, it } from "vitest";
import { compactWindow, usageHelp } from "./usage";

it("chooses five-hour usage for Claude and weekly usage for Codex, regardless of order", () => {
  const weekly = { label: "semana", usedPct: 20, resetsAt: null };
  const fiveHour = { label: "5 h", usedPct: 6, resetsAt: null };
  const claude = { provider: "claude", name: "Claude", windows: [weekly, fiveHour] };
  const codex = { provider: "codex", name: "Codex", windows: [fiveHour, weekly] };
  expect(compactWindow(claude)).toEqual(fiveHour);
  expect(compactWindow(codex)).toEqual(weekly);
  expect(usageHelp(codex, weekly)).toBe("Codex · semana · 20 % usado");
});

it("uses available data for other allowances and omits empty plans", () => {
  const monthly = { label: "mes", usedPct: 35, resetsAt: null };
  expect(compactWindow({ provider: "antigravity", name: "Gemini", windows: [monthly] })).toEqual(monthly);
  expect(compactWindow({ provider: "codex", name: "Codex", windows: [] })).toBeUndefined();
});
