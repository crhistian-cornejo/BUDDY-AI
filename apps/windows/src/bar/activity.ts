export interface ActivitySession { agent: string; project: string; state: string }
export interface ActivityState {
  sessions: ActivitySession[];
  buddyBusy: boolean;
  buddyLabel?: string;
  focusing: boolean;
  playing: boolean;
  player?: string;
}

export function activeSession(sessions: ActivitySession[]) {
  return sessions.find((s) => s.state === "waiting") ?? sessions.find((s) => s.state === "working");
}

export function compactActivity(state: ActivityState): "session" | "buddy" | "focus" | "music" | "none" {
  if (activeSession(state.sessions)?.state === "waiting") return "session";
  if (state.buddyBusy) return "buddy";
  if (activeSession(state.sessions)) return "session";
  if (state.focusing) return "focus";
  if (state.playing) return "music";
  return "none";
}

export function activityLabel(state: ActivityState, agentName: (id: string) => string): string {
  const session = activeSession(state.sessions);
  if (session?.state === "waiting") return `${agentName(session.agent)} espera tu respuesta`;
  if (state.buddyBusy) return state.buddyLabel ?? "Buddy está respondiendo";
  const working = state.sessions.filter((s) => s.state === "working").length;
  if (working) return working === 1 ? "1 agente trabajando" : `${working} agentes trabajando`;
  if (state.focusing) return "Enfoque en curso";
  if (state.playing) return `Sonando en ${state.player}`;
  return "Todo a mano";
}
