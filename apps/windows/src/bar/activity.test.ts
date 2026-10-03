import { describe, expect, it } from "vitest";
import { compactActivity, activityLabel, type ActivityState } from "./activity";

const base: ActivityState = { sessions: [], buddyBusy: false, focusing: false, playing: false };
const agentName = (id: string) => id === "codex" ? "Codex" : "Claude Code";

describe("notch live activities", () => {
  it("prioritizes waiting sessions above Buddy, focus and music", () => {
    const state = { ...base, buddyBusy: true, focusing: true, playing: true,
      sessions: [{ agent: "claude", project: "A", state: "working" }, { agent: "codex", project: "B", state: "waiting" }] };
    expect(compactActivity(state)).toBe("session");
    expect(activityLabel(state, agentName)).toBe("Codex espera tu respuesta");
  });

  it("shows Buddy before working sessions and restores the session after the turn", () => {
    const state = { ...base, buddyBusy: true, buddyLabel: "Revisando archivos",
      sessions: [{ agent: "codex", project: "Buddy", state: "working" }] };
    expect(compactActivity(state)).toBe("buddy");
    expect(activityLabel(state, agentName)).toBe("Revisando archivos");
    state.buddyBusy = false;
    expect(compactActivity(state)).toBe("session");
    expect(activityLabel(state, agentName)).toBe("1 agente trabajando");
  });

  it("falls back to focus, music and idle without showing completed sessions", () => {
    const state = { ...base, focusing: true, playing: true, player: "Spotify",
      sessions: [{ agent: "codex", project: "Buddy", state: "done" }] };
    expect(compactActivity(state)).toBe("focus");
    state.focusing = false;
    expect(compactActivity(state)).toBe("music");
    expect(activityLabel(state, agentName)).toBe("Sonando en Spotify");
    state.playing = false;
    expect(compactActivity(state)).toBe("none");
    expect(activityLabel(state, agentName)).toBe("Todo a mano");
  });
});
