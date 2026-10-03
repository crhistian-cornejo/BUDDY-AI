export interface UsageWindow { label: string; usedPct: number; resetsAt: number | null }
export interface ProviderUsage { provider: string; name: string; windows: UsageWindow[] }

/** Show Codex's week and other providers' five-hour window, falling back to available data. */
export function compactWindow(plan: ProviderUsage): UsageWindow | undefined {
  const fiveHour = plan.windows.find((w) => w.label.toLowerCase() === "5 h");
  const weekly = plan.windows.find((w) => w.label.toLowerCase() === "semana");
  return (plan.provider === "codex" ? weekly ?? fiveHour : fiveHour ?? weekly) ?? plan.windows[0];
}

export function usageHelp(plan: ProviderUsage, window: UsageWindow): string {
  const reset = window.resetsAt ? ` · se reinicia ${new Date(window.resetsAt * 1000).toLocaleString("es", {
    weekday: "short", day: "numeric", hour: "2-digit", minute: "2-digit",
  })}` : "";
  return `${plan.name} · ${window.label} · ${Math.round(window.usedPct)} % usado${reset}`;
}
