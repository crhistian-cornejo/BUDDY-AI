// The top bar (Windows), twin of the Mac notch: one notice at a time (a permission of Claude Code or Codex with
// Permitir / Rechazar, a session that finished or waits, the end of a focus block), and on hover the tools: the music
// player, the agents, the focus timer and the pinned shortcuts. Files dragged onto it can go to Buddy. The window is
// always exactly the island's size, so nothing around it is covered.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { h, svg } from "../chat/dom";
import { TABLER } from "../chat/tabler";
import { PROVIDER_MARKS } from "../chat/provider-marks";

type Kind = "approval" | "finished" | "waiting" | "failed";
interface Notice { kind: Kind; agent: string; title: string; detail: string; command?: string; requestId?: string; canAllow?: boolean }
interface Session { id: string; agent: string; project: string; state: string }
interface NowPlaying { app: string; title: string; artist: string; status: string; positionMs: number | null; durationMs: number | null; thumbnail: string | null }
interface FocusStatus { running: boolean; startedAt: number; endsAt: number; minutes: number }
interface Shortcut { id: string; name: string; target: string; kind: string }
interface UsageWindow { label: string; usedPct: number; resetsAt: number | null }
interface ProviderUsage { provider: string; name: string; windows: UsageWindow[] }
type CoreEvent =
  | { type: "approvalRequest"; requestId: string; sessionId: string; agent: string; project: string; title: string; summary: string; detail: string; canAllow: boolean }
  | { type: "approvalClosed"; requestId: string }
  | { type: "sessionUpdate"; sessionId: string; agent: string; project: string; state: string; cwd: string; terminal: string; summary: string }
  | { type: "focusChanged"; running: boolean; endsAt: number }
  | { type: "focusFinished"; minutes: number }
  | { type: "mascotState"; state: string }
  | { type: "usageChanged" }
  | { type: "usageLow"; provider: string; label: string; leftPct: number }
  | { type: string };

const $ = (id: string) => document.getElementById(id)!;
const island = $("island");
const ears = $("ears");
const noticeEl = $("notice");
const overview = $("overview");
const dropEl = $("drop");

const ICON = { fill: "none", stroke: "currentColor", "stroke-width": "1.75", "stroke-linecap": "round", "stroke-linejoin": "round" };
const icon = (path: string, size = 16) => svg(path, size, ICON);
const PILL = { width: 160, height: 8 };
const EARS = { width: 220, height: 30 };
const NOTICE_SECONDS = 6;
const WIDTH = { notice: 420, open: 560, drop: 420 } as const;

let notice: Notice | null = null;
const queue: Notice[] = [];
let sessions: Session[] = [];
let hovering = false;
let dismissTimer = 0;
let track: NowPlaying | null = null;
let trackAt = Date.now();
let focus: FocusStatus | null = null;
let shortcuts: Shortcut[] = [];
let dropped: string[] = [];
let dragging = false;
let tick = 0;
let hooksConnected = false;
/** Buddy is answering in the chat (shown beside the bar while the chat is closed). */
let buddyBusy = false;
let usage: ProviderUsage[] = [];
const pendingApproval = new Map<string, string>();

const agentName = (agent: string) => (agent === "codex" ? "Codex" : agent === "buddy" ? "Buddy" : "Claude Code");

function providerMark(agent: string, size: number): Element {
  const mark = PROVIDER_MARKS[agent === "codex" ? "codex" : "claude"]!;
  const el = h("span", { title: mark.label, style: `display:inline-grid;color:${mark.color ?? "#fafafa"}` });
  el.append(svg(mark.path, size, { fill: "currentColor" }));
  return el;
}

const activeSession = () => sessions.find((s) => s.state === "waiting") ?? sessions.find((s) => s.state === "working");
const mode = (): "notice" | "drop" | "open" | "idle" =>
  notice ? "notice" : dragging || dropped.length ? "drop" : hovering ? "open" : "idle";
const left = () => Math.max((focus?.endsAt ?? 0) - Date.now() / 1000, 0);
const clock = (seconds: number) => `${Math.floor(seconds / 60)}:${String(Math.floor(seconds % 60)).padStart(2, "0")}`;
const stateText = (state: string) =>
  ({ working: "Trabajando", waiting: "Esperando tu respuesta", error: "Terminó con un error" })[state] ?? "Terminó";

/** Redraws and asks Rust to size the window to the island. */
function render() {
  const m = mode();
  island.className = m;
  noticeEl.hidden = m !== "notice";
  overview.hidden = m !== "open";
  dropEl.hidden = m !== "drop";
  const active = activeSession();
  const focusing = !!focus?.running;
  const ear = earKind();
  ears.hidden = m !== "idle" || ear === "none";
  if (m === "idle") drawEars(active);
  if (m === "notice") drawNotice();
  if (m === "open") drawOverview();
  if (m === "drop") drawDrop();
  // A countdown or a playing track redraws once a second, only while it is on screen.
  window.clearInterval(tick);
  if ((m === "open" && (focusing || track?.status === "playing")) || (m === "idle" && ear === "focus")) {
    tick = window.setInterval(() => (m === "open" ? drawOverview() : drawEars(active)), 1000);
  }
  requestAnimationFrame(() => {
    const size = m === "idle" ? (ear !== "none" ? EARS : PILL) : { width: WIDTH[m], height: Math.ceil(island.scrollHeight) };
    void invoke("bar_resize", size);
  });
}

/** What sits beside the bar at rest, by priority: an agent, Buddy answering, the focus countdown, the music playing. */
function earKind(): "session" | "buddy" | "focus" | "music" | "none" {
  if (activeSession()) return "session";
  if (buddyBusy) return "buddy";
  if (focus?.running) return "focus";
  if (track?.status === "playing") return "music";
  return "none";
}

function drawEars(active: Session | undefined) {
  const kind = earKind();
  if (kind === "buddy") {
    $("ear-left").replaceChildren(h("span", { style: "display:inline-grid" }, icon(TABLER.sparkles, 14)));
    $("ear-right").replaceChildren(h("span", { class: "thinking" }, h("i"), h("i"), h("i")));
    ears.title = "Buddy está respondiendo";
    return;
  }
  if (kind === "music" && track) {
    $("ear-left").replaceChildren(h("span", { style: "color:#22c55e;display:inline-grid" }, icon(TABLER.musicNote, 14)));
    $("ear-right").replaceChildren(h("span", { class: "eq" }, h("i"), h("i"), h("i")));
    ears.title = `${track.title} · ${track.artist}`;
    return;
  }
  if (active) {
    $("ear-left").replaceChildren(providerMark(active.agent, 14));
    $("ear-right").replaceChildren(h("span", { class: `dot ${active.state}` }));
    ears.title = `${agentName(active.agent)} · ${active.project}`;
  } else if (focus) {
    $("ear-left").replaceChildren(h("span", { style: "color:#f59e0b;display:inline-grid" }, icon(TABLER.history, 14)));
    $("ear-right").replaceChildren(h("span", { style: "color:#f59e0b;font-weight:600;font-size:11px", text: `${Math.ceil(left() / 60)}m` }));
    ears.title = "Enfoque";
  }
}

function drawNotice() {
  if (!notice) return;
  const n = notice;
  const mark = n.agent === "buddy" ? icon(TABLER.sparkles, 18) : providerMark(n.agent, 18);
  const head = h("div", { class: "notice-head" },
    h("span", { class: "mark" }, mark),
    h("div", { class: "notice-text" }, h("strong", { text: n.title }), h("span", { text: n.detail })));
  const parts: Node[] = [head];
  if (n.command) parts.push(h("pre", { class: "command", text: n.command }));
  if (n.kind === "approval" && n.requestId) {
    const id = n.requestId;
    const row = h("div", { class: "actions" });
    if (!n.canAllow) row.append(h("span", { class: "muted", text: "Es demasiado largo para revisarlo aquí: respóndelo en la terminal." }));
    row.append(h("button", { class: "btn", type: "button", title: `No permitirlo; ${agentName(n.agent)} seguirá sin hacerlo`, onclick: () => answer(id, false) }, "Rechazar"));
    if (n.canAllow) row.append(h("button", { class: "btn primary", type: "button", title: "Permitir esta vez", onclick: () => answer(id, true) }, "Permitir"));
    parts.push(row);
  }
  noticeEl.replaceChildren(...parts);
  noticeEl.onclick = n.kind === "approval" ? null : () => dismiss();
}

function drawOverview() {
  drawMusic();
  drawAgents();
  drawFocus();
  drawShortcuts();
  drawUsage();
}

/** What is used of each plan: one column per provider, a row per window under one another. */
function drawUsage() {
  const el = $("usage");
  el.hidden = !usage.length;
  el.replaceChildren(...usage.map((plan) => h("div", { class: "plan" }, providerMark(plan.provider, 12),
    h("div", { class: "windows" }, ...plan.windows.slice(0, 3).map((w) => {
      const pct = Math.round(w.usedPct);
      const reset = w.resetsAt ? ` · se reinicia ${new Date(w.resetsAt * 1000).toLocaleString("es", { weekday: "short", hour: "2-digit", minute: "2-digit" })}` : "";
      const level = pct >= 90 ? "high" : pct >= 70 ? "warn" : "";
      return h("div", { class: "window", title: `${pct} % usado (${w.label})${reset}` },
        h("span", { class: "muted label", text: w.label }),
        h("div", { class: "meter" }, h("i", { class: level, style: `width:${Math.min(Math.max(pct, 0), 100)}%` })),
        h("span", { class: "pct", text: `${pct} %` }));
    })))));
}

function drawMusic() {
  const music = $("music");
  music.hidden = !track;
  if (!track) return;
  const t = track;
  const playing = t.status === "playing";
  const control = (path: string, title: string, action: string, size: number) =>
    h("button", { class: "icon-btn", type: "button", title, "aria-label": title, onclick: () => void invoke("media_control", { action }) }, icon(path, size));
  const info: Node[] = [h("strong", { text: t.title }), h("span", { class: "muted", text: [t.artist, t.app].filter(Boolean).join(" · ") })];
  if (t.durationMs && t.positionMs !== null) {
    const now = Math.min(t.positionMs + (playing ? Date.now() - trackAt : 0), t.durationMs);
    const fill = h("div", { class: "bar-fill", style: `width:${(now / t.durationMs) * 100}%` });
    info.push(h("div", { class: "progress" }, h("span", { text: clock(now / 1000) }), h("div", { class: "bar-track" }, fill),
      h("span", { text: `-${clock((t.durationMs - now) / 1000)}` })));
  }
  music.replaceChildren(
    t.thumbnail ? h("img", { src: t.thumbnail, alt: "" }) : h("span", { class: "art" }),
    h("div", { class: "track" }, ...info),
    h("div", { class: "controls" },
      control(TABLER.playerSkipBack, "Anterior", "previous", 16),
      control(playing ? TABLER.playerPause : TABLER.playerPlay, playing ? "Pausar" : "Reproducir", "play_pause", 20),
      control(TABLER.playerSkipForward, "Siguiente", "next", 16)));
}

function drawAgents() {
  const rows: Node[] = sessions.slice(0, 3).map((s) =>
    h("div", { class: "session", title: stateText(s.state) }, providerMark(s.agent, 11), h("span", { class: "label", text: s.project }), h("span", { class: `dot ${s.state}` })));
  if (!sessions.length) {
    rows.push(h("span", { class: "muted", text: hooksConnected ? "Sin sesiones abiertas" : "Claude Code y Codex" }));
    if (!hooksConnected) {
      rows.push(h("div", { class: "row-btns" },
        h("button", { class: "pill primary", type: "button", title: "Muestra qué cambia en su configuración antes de hacerlo",
          onclick: () => void invoke("connect_hooks").then(refreshHooks) }, "Conectar")));
    }
  }
  $("agents").replaceChildren(h("h2", {}, icon(TABLER.terminal2, 14), "Agentes"), ...rows);
}

function drawFocus() {
  const body: Node[] = [];
  if (focus?.running) {
    const total = Math.max(focus.endsAt - focus.startedAt, 1);
    const r = 20;
    const c = 2 * Math.PI * r;
    const ns = "http://www.w3.org/2000/svg";
    const ring = document.createElementNS(ns, "svg");
    ring.setAttribute("width", "46");
    ring.setAttribute("height", "46");
    for (const [stroke, dash] of [["rgba(255,255,255,0.15)", c], ["#f59e0b", (left() / total) * c]] as const) {
      const circle = document.createElementNS(ns, "circle");
      const attrs = { cx: "23", cy: "23", r: String(r), fill: "none", stroke, "stroke-width": "4", "stroke-linecap": "round", "stroke-dasharray": `${dash} ${c}` };
      for (const [k, v] of Object.entries(attrs)) circle.setAttribute(k, v);
      ring.append(circle);
    }
    body.push(h("div", { class: "row-btns" },
      h("div", { class: "ring" }, ring, h("span", { text: clock(left()) })),
      h("button", { class: "pill", type: "button", title: "Terminar el bloque de enfoque ahora", onclick: () => void invoke("focus_stop") }, "Parar")));
  } else {
    body.push(h("span", { class: "muted", text: "Sin distracciones" }),
      h("div", { class: "row-btns" },
        h("button", { class: "pill primary", type: "button", title: "Empezar 25 minutos de enfoque", onclick: () => void invoke("focus_start", { minutes: 25 }) }, "25 min"),
        h("button", { class: "pill", type: "button", title: "Empezar 50 minutos de enfoque", onclick: () => void invoke("focus_start", { minutes: 50 }) }, "50")));
  }
  $("focus").replaceChildren(h("h2", {}, icon(TABLER.history, 14), "Enfoque"), ...body);
}

function drawShortcuts() {
  const grid = h("div", { class: "grid" });
  for (const s of shortcuts) {
    const glyph = s.kind === "web" ? TABLER.arrowUpRight : s.kind === "folder" ? TABLER.folder : s.kind === "app" ? TABLER.terminal2 : TABLER.fileText;
    const b = h("button", { class: "shortcut", type: "button", title: s.name, "aria-label": s.name,
      onclick: () => { hovering = false; render(); void invoke("open_shortcut", { target: s.target }); } }, icon(glyph, 16));
    b.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      if (confirm(`¿Quitar «${s.name}» de Atajos?`)) {
        void invoke<Shortcut[]>("remove_shortcut", { id: s.id }).then((l) => { shortcuts = l; render(); });
      }
    });
    grid.append(b);
  }
  if (shortcuts.length < 8) {
    grid.append(h("button", { class: "shortcut", type: "button", title: "Fijar una app o un archivo", "aria-label": "Fijar un atajo",
      onclick: () => void invoke<Shortcut[]>("pick_shortcut").then((l) => { shortcuts = l; render(); }) }, icon(TABLER.edit, 16)));
  }
  $("shortcuts").replaceChildren(h("h2", {}, icon(TABLER.folder, 14), "Atajos"), grid);
}

function drawDrop() {
  if (!dropped.length) {
    dropEl.replaceChildren(h("div", { class: "drop-target" }, icon(TABLER.folder, 22), h("span", { text: "Suelta tus archivos aquí" })));
    return;
  }
  const name = (p: string) => p.split(/[\\/]/).pop() ?? p;
  const files = dropped;
  dropEl.replaceChildren(
    h("div", { class: "drop-file" }, icon(TABLER.folder, 28),
      h("div", { class: "names" }, h("strong", { text: files.length === 1 ? name(files[0]!) : `${files.length} archivos` }),
        h("span", { class: "muted", text: files.length === 1 ? files[0]! : files.map(name).join(", ") })),
      h("button", { class: "icon-btn", type: "button", title: "Descartar", onclick: () => { dropped = []; render(); } }, icon(TABLER.x, 16))),
    h("div", { class: "actions", style: "justify-content:flex-start" },
      h("button", { class: "btn primary", type: "button", title: "Abre el chat con los archivos",
        onclick: () => { void invoke("give_files", { paths: files }); dropped = []; render(); } }, "Dárselo a Buddy"),
      h("button", { class: "btn", type: "button", title: "Muestra el archivo en el Explorador", onclick: () => void invoke("reveal_path", { path: files[0] }) }, "Mostrar en carpeta"),
      h("button", { class: "btn", type: "button", title: "Copia la ruta al portapapeles",
        onclick: () => { void navigator.clipboard.writeText(files.join("\n")); dropped = []; render(); } }, "Copiar ruta")));
}

function show(n: Notice) {
  if (!notice) present(n);
  else if (n.kind === "approval" && notice.kind !== "approval") present(n);
  else if (n.kind === "approval") {
    const at = queue.findIndex((q) => q.kind !== "approval");
    queue.splice(at < 0 ? queue.length : at, 0, n);
  } else queue.push(n);
}

function present(n: Notice) {
  notice = n;
  window.clearTimeout(dismissTimer);
  if (n.kind !== "approval") dismissTimer = window.setTimeout(() => { if (!hovering) dismiss(); }, NOTICE_SECONDS * 1000);
  render();
}

function dismiss() {
  window.clearTimeout(dismissTimer);
  notice = null;
  const next = queue.shift();
  if (next) present(next);
  else render();
}

function answer(requestId: string, allow: boolean) {
  void invoke("answer_approval", { requestId, allow });
  closeApproval(requestId);
}

function closeApproval(requestId: string) {
  for (let i = queue.length - 1; i >= 0; i--) if (queue[i]!.requestId === requestId) queue.splice(i, 1);
  if (notice?.requestId === requestId) dismiss();
}

function onCore(e: CoreEvent) {
  switch (e.type) {
    case "approvalRequest": {
      const a = e as Extract<CoreEvent, { type: "approvalRequest" }>;
      pendingApproval.set(a.requestId, a.sessionId);
      show({ kind: "approval", agent: a.agent, requestId: a.requestId, canAllow: a.canAllow,
        title: `${agentName(a.agent)} pide permiso en ${a.project}`, detail: `${a.title}: ${a.summary}`, command: a.detail });
      break;
    }
    case "approvalClosed": {
      const id = (e as Extract<CoreEvent, { type: "approvalClosed" }>).requestId;
      pendingApproval.delete(id);
      closeApproval(id);
      break;
    }
    case "usageChanged":
      void invoke<ProviderUsage[]>("usage").then((u) => { usage = u; if (mode() === "open") render(); });
      break;
    case "usageLow": {
      const u = e as Extract<CoreEvent, { type: "usageLow" }>;
      const name = u.provider === "codex" ? "Codex" : "Claude";
      show({ kind: "waiting", agent: u.provider, title: `Te queda ${u.leftPct} % de ${name}`, detail: `Ventana: ${u.label}. Buddy usará el otro proveedor si se acaba.` });
      break;
    }
    case "mascotState": {
      const state = (e as Extract<CoreEvent, { type: "mascotState" }>).state;
      const busy = state === "think" || state === "work";
      if (busy !== buddyBusy) { buddyBusy = busy; render(); }
      break;
    }
    case "focusChanged":
      void invoke<FocusStatus>("focus_status").then((f) => { focus = f.running ? f : null; render(); });
      break;
    case "focusFinished": {
      const minutes = (e as Extract<CoreEvent, { type: "focusFinished" }>).minutes;
      focus = null;
      show({ kind: "finished", agent: "buddy", title: "Terminó tu bloque de enfoque", detail: `${minutes} minutos. Tómate un respiro.` });
      break;
    }
    case "sessionUpdate": {
      const s = e as Extract<CoreEvent, { type: "sessionUpdate" }>;
      const old = sessions.find((x) => x.id === s.sessionId);
      const before = old?.state;
      if (s.state === "ended") sessions = sessions.filter((x) => x.id !== s.sessionId);
      else if (old) old.state = s.state;
      else sessions.unshift({ id: s.sessionId, agent: s.agent, project: s.project, state: s.state });
      if (before !== s.state) {
        const name = agentName(s.agent);
        if (s.state === "done") show({ kind: "finished", agent: s.agent, title: `${name} terminó en ${s.project}`, detail: s.summary || "Terminó su turno." });
        if (s.state === "error") show({ kind: "failed", agent: s.agent, title: `${name} se detuvo por un error`, detail: s.project });
        if (s.state === "waiting") {
          // The permission request comes a moment after this state: wait for it before saying anything.
          window.setTimeout(() => {
            const still = sessions.find((x) => x.id === s.sessionId)?.state === "waiting";
            if (still && ![...pendingApproval.values()].includes(s.sessionId)) {
              show({ kind: "waiting", agent: s.agent, title: `${name} espera tu respuesta`, detail: s.project });
            }
          }, 300);
        }
      }
      render();
      break;
    }
  }
}

async function refreshHooks() {
  const status = await invoke<{ installed: boolean }[]>("hooks_status").catch(() => []);
  hooksConnected = status.some((s) => s.installed);
  if (mode() === "open") render();
}

island.addEventListener("mouseenter", () => {
  hovering = true;
  window.clearTimeout(dismissTimer);
  if (!notice) {
    void refreshHooks();
    void invoke("refresh_usage");
  }
  render();
});
island.addEventListener("mouseleave", () => {
  hovering = false;
  if (notice && notice.kind !== "approval") dismissTimer = window.setTimeout(dismiss, 2000);
  render();
});

// Files dragged onto the bar.
void getCurrentWebview().onDragDropEvent(({ payload }) => {
  if (payload.type === "enter" || payload.type === "over") {
    if (!dragging) { dragging = true; render(); }
  } else if (payload.type === "leave") {
    dragging = false;
    render();
  } else if (payload.type === "drop") {
    dragging = false;
    dropped = payload.paths;
    render();
  }
});

void listen<CoreEvent>("core-event", ({ payload }) => onCore(payload));
// Windows raises its own media events, so the bar listens all the time (the worker sleeps between them: no polling).
void listen<NowPlaying | null>("media-changed", ({ payload }) => { track = payload; trackAt = Date.now(); render(); });
void invoke("media_watch", { on: true });
void invoke<NowPlaying | null>("media_now_playing").then((t) => { track = t; trackAt = Date.now(); render(); });
void Promise.all([
  invoke<{ sessionId: string; agent: string; project: string; state: string; cwd: string; terminal: string; summary: string }[]>("sessions").catch(() => []),
  invoke<FocusStatus>("focus_status").catch(() => null),
  invoke<Shortcut[]>("shortcuts").catch(() => []),
  invoke<ProviderUsage[]>("usage").catch(() => []),
]).then(([list, f, s, u]) => {
  usage = u;
  sessions = list.map((x) => ({ id: x.sessionId, agent: x.agent, project: x.project, state: x.state }));
  focus = f?.running ? f : null;
  shortcuts = s;
  void refreshHooks();
  render();
});
