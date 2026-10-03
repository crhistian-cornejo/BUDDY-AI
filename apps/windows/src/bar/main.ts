import { YouTubePanel } from "./youtube";
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
import { agentFace } from "../chat/avatar";
import { activeSession as findActiveSession, compactActivity, activityLabel } from "./activity";
import { compactWindow, usageHelp, type ProviderUsage } from "./usage";
import { NotchToolPanel } from "./tools";
import { lockUntilRead } from "./approval";

type Kind = "approval" | "finished" | "waiting" | "failed";
/** `question`: an agent asks the user something (Codex's question tool); the card opens by itself with the question. */
interface Notice { kind: Kind; agent: string; title: string; detail: string; command?: string; requestId?: string; canAllow?: boolean; always?: string; question?: boolean }
interface Session { id: string; agent: string; project: string; state: string }
interface NowPlaying { app: string; title: string; artist: string; status: string; positionMs: number | null; durationMs: number | null; thumbnail: string | null }
interface FocusStatus { running: boolean; startedAt: number; endsAt: number; minutes: number }
interface Shortcut { id: string; name: string; target: string; kind: string }
type CoreEvent =
  | { type: "approvalRequest"; requestId: string; sessionId: string; agent: string; project: string; title: string; summary: string; detail: string; canAllow: boolean; always: string }
  | { type: "approvalClosed"; requestId: string }
  | { type: "sessionUpdate"; sessionId: string; agent: string; project: string; state: string; cwd: string; terminal: string; summary: string }
  | { type: "focusChanged"; running: boolean; endsAt: number }
  | { type: "focusFinished"; minutes: number }
  | { type: "mascotState"; state: string }
  | { type: "usageChanged" }
  | { type: "usageLow"; provider: string; label: string; leftPct: number }
  | { type: "financeRecorded"; monto: string; moneda: string; tipo: string; concepto: string; comercio: string }
  | { type: "budgetAlert"; categoria: string; usadoPct: number }
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
const NOTICE_EARS = { width: 304, height: 30 };
const NOTICE_SECONDS = 6;
const WIDTH = { notice: 420, open: 560, drop: 420 } as const;

let notice: Notice | null = null;
const queue: Notice[] = [];
let sessions: Session[] = [];
let hovering = false;
let pinned = false;
let collapsedByUser = false;
let hoverTimer = 0;
let leaveTimer = 0;
let dismissTimer = 0;
let track: NowPlaying | null = null;
let trackAt = Date.now();
let focus: FocusStatus | null = null;
let shortcuts: Shortcut[] = [];
let dragging = false;
let tick = 0;
let hooksConnected = false;
/** Buddy is answering in the chat (shown beside the bar while the chat is closed). */
let buddyBusy = false;
/** What Buddy's turn is doing (the core's ChatActivity): the ear shows its icon. */
let buddyActivity: { kind: string; label: string } | null = null;
let usage: ProviderUsage[] = [];
const pendingApproval = new Map<string, string>();
/** Approval cards: where the reader is in each command, and which ones were seen to their end (by request id). */
const commandScroll = new Map<string, number>();
const commandRead = new Set<string>();
const tools = new NotchToolPanel(render);
const youtube = new YouTubePanel(started => { if (started) { pinned = true; collapsedByUser = false; tools.tab = "home"; } render(); });

const agentName = (agent: string) =>
  agent === "codex" ? "Codex" : agent === "antigravity" ? "Gemini" : agent === "buddy" ? "Buddy" : agent === "niko" ? "Niko" : "Claude Code";
/** A plan's name, as the Mac's AgentNames.plan. */
const planName = (provider: string) => (provider === "codex" ? "Codex" : provider === "antigravity" ? "Gemini" : "Claude");

function providerMark(agent: string, size: number): Element {
  const mark = PROVIDER_MARKS[agent === "codex" || agent === "antigravity" ? agent : "claude"]!;
  const el = h("span", { title: mark.label, style: `display:inline-grid;color:${mark.color ?? "#fafafa"}` });
  el.append(svg(mark.path, size, { fill: "currentColor" }));
  return el;
}

const activeSession = () => findActiveSession(sessions) as Session | undefined;
const activityState = () => ({ sessions, buddyBusy, buddyLabel: buddyActivity?.label,
  focusing: !!focus?.running, playing: youtube.state.detected?.playing || track?.status === "playing", player: youtube.state.detected?.playing ? "YouTube" : track?.app });
const mode = (): "notice" | "drop" | "open" | "idle" =>
  notice && (notice.kind === "approval" || notice.question || pinned || (hovering && !collapsedByUser)) ? "notice"
    : dragging ? "drop" : pinned || (hovering && !collapsedByUser) ? "open" : "idle";
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
  tools.draw(m === "open");
  youtube.draw(m === "open" && tools.tab === "home");
  if (youtube.state.viewer && youtube.state.destination === "notch" && m === "open" && tools.tab === "home") $("home-tools").hidden = true;
  const active = activeSession();
  const focusing = !!focus?.running;
  const ear = earKind();
  ears.classList.toggle("compact-notice", ear === "notice");
  ears.hidden = m !== "idle" || ear === "none";
  if (m === "idle") drawEars(active);
  if (m === "notice") drawNotice();
  if (m === "open") drawOverview();
  if (m === "drop") drawDrop();
  // A countdown or a playing track redraws once a second, only while it is on screen.
  window.clearInterval(tick);
  if ((m === "open" && (focusing || track?.status === "playing")) || (m === "idle" && ear === "focus")) {
    tick = window.setInterval(() => (m === "open" ? updateTimers() : drawEars(active)), 1000);
  }
  requestAnimationFrame(() => {
    const size = m === "idle" ? (ear === "notice" ? NOTICE_EARS : ear !== "none" ? EARS : PILL) : { width: WIDTH[m], height: Math.ceil(island.scrollHeight) };
    void invoke("bar_resize", size);
  });
}

/** What sits beside the bar at rest, by priority: an agent, Buddy answering, the focus countdown, the music playing. */
function earKind(): "notice" | "session" | "buddy" | "focus" | "music" | "none" {
  if (notice && notice.kind !== "approval") return "notice";
  return compactActivity(activityState());
}

/** Tabler paths (MIT) and colours for what Buddy is doing: Word blue, Excel green, PowerPoint orange… */
const ACTIVITY: Record<string, [string, string]> = {
  word: [TABLER.fileText, "#4f8be0"],
  excel: ["M3 5a2 2 0 0 1 2 -2h14a2 2 0 0 1 2 2v14a2 2 0 0 1 -2 2h-14a2 2 0 0 1 -2 -2z M3 10h18 M10 3v18", "#3fb97a"],
  powerpoint: ["M3 4h18 M4 4v10a2 2 0 0 0 2 2h12a2 2 0 0 0 2 -2v-10 M12 16v4 M9 20h6 M8 12l3 -3l2 2l3 -3", "#f0784a"],
  web: [TABLER.world, "var(--mint)"],
  read: [TABLER.search, "#fafafa"],
  command: ["M5 7l5 5l-5 5 M12 19h7", "#fafafa"],
  edit: [TABLER.pencil, "#fafafa"],
  screen: ["M10 12a2 2 0 1 0 4 0a2 2 0 0 0 -4 0 M21 12c-2.4 4 -5.4 6 -9 6c-3.6 0 -6.6 -2 -9 -6c2.4 -4 5.4 -6 9 -6c3.6 0 6.6 2 9 6", "var(--indigo)"],
  music: [TABLER.musicNote, "var(--mint)"],
  skill: [TABLER.sparkles, "var(--indigo)"],
};

function drawEars(active: Session | undefined) {
  const kind = earKind();
  if (kind === "notice" && notice) {
    const mark = notice.agent === "buddy" || notice.agent === "niko" ? icon(TABLER.sparkles, 14) : providerMark(notice.agent, 14);
    $("ear-left").replaceChildren(mark, h("span", { text: agentName(notice.agent) }));
    $("ear-right").replaceChildren(h("span", { class: `notice-state ${notice.kind}`,
      text: notice.kind === "finished" ? "Listo" : notice.kind === "failed" ? "Error" : "Espera" }));
    ears.title = `${notice.title} · ${notice.detail}`;
    return;
  }
  if (kind === "buddy") {
    $("ear-left").replaceChildren(h("span", { style: "display:inline-grid" }, icon(TABLER.sparkles, 14)));
    const doing = buddyActivity && ACTIVITY[buddyActivity.kind];
    $("ear-right").replaceChildren(doing
      ? h("span", { class: `activity-glyph ${buddyActivity!.kind}`, style: `color:${doing[1]}` }, icon(doing[0], 14))
      : h("span", { class: "thinking" }, h("i"), h("i"), h("i")));
    ears.title = buddyActivity?.label ?? "Buddy está respondiendo";
    return;
  }
  if (kind === "music" && (track || youtube.state.detected)) {
    $("ear-left").replaceChildren(h("span", { style: "color:var(--mint);display:inline-grid" }, icon(TABLER.musicNote, 14)));
    $("ear-right").replaceChildren(h("span", { class: "eq" }, h("i"), h("i"), h("i")));
    const video = youtube.state.detected;
    ears.title = video?.playing ? `${video.title} · YouTube` : `${track?.title ?? ''} · ${track?.artist ?? ''}`;
    return;
  }
  if (active) {
    $("ear-left").replaceChildren(providerMark(active.agent, 14));
    $("ear-right").replaceChildren(h("span", { class: `dot ${active.state}` }));
    ears.title = `${agentName(active.agent)} · ${active.project}`;
  } else if (focus) {
    $("ear-left").replaceChildren(h("span", { style: "color:var(--indigo);display:inline-grid" }, icon(TABLER.history, 14)));
    $("ear-right").replaceChildren(h("span", { style: "color:var(--indigo);font-weight:600;font-size:11px", text: `${Math.ceil(left() / 60)}m` }));
    ears.title = "Enfoque";
  }
}

function drawNotice() {
  if (!notice) return;
  const n = notice;
  let mark: Element = n.agent === "buddy" ? icon(TABLER.sparkles, 18) : providerMark(n.agent, 18);
  if (n.agent === "niko") {
    // Niko's own pixel face (the core paints it); a sparkle until it is ready.
    const holder = h("span", {}, icon(TABLER.sparkles, 18));
    void agentFace("niko").then((url) => { if (url) holder.replaceChildren(h("img", { src: url, alt: "", width: 22, height: 22, style: "image-rendering: pixelated" })); });
    mark = holder;
  }
  const head = h("div", { class: "notice-head" },
    h("span", { class: "mark" }, mark),
    h("div", { class: "notice-text" }, h("strong", { text: n.title }), h("span", { text: n.detail })));
  const parts: Node[] = [head];
  const pre = n.command ? h("pre", { class: "command", text: n.command }) as HTMLElement : null;
  if (pre) parts.push(pre);
  // The buttons that allow: off until the whole command has been in view.
  const allow: HTMLButtonElement[] = [];
  const unread = h("span", { class: "muted", text: "Hay más abajo: desplázate hasta el final para poder permitir." }) as HTMLElement;
  if (n.kind === "approval" && n.requestId) {
    const id = n.requestId;
    const row = h("div", { class: "actions" });
    if (!n.canAllow) row.append(h("span", { class: "muted", text: "Es demasiado largo para revisarlo aquí: respóndelo en la terminal." }));
    else if (pre) row.append(unread);
    row.append(h("button", { class: "btn", type: "button", title: `No permitirlo; ${agentName(n.agent)} seguirá sin hacerlo`, onclick: () => answer(id, false) }, "Rechazar"));
    if (n.canAllow && n.always) {
      allow.push(h("button", { class: "btn", type: "button", title: `Permitir siempre «${n.always} …» a ${agentName(n.agent)}; se quita en Ajustes › General`, onclick: () => answerAlways(id) }, "Permitir siempre") as HTMLButtonElement);
    }
    if (n.canAllow) allow.push(h("button", { class: "btn primary", type: "button", title: "Permitir esta vez", onclick: () => answer(id, true) }, "Permitir") as HTMLButtonElement);
    row.append(...allow);
    parts.push(row);
  }
  noticeEl.replaceChildren(...parts);
  if (pre && n.requestId) {
    const id = n.requestId;
    // A redraw keeps the reader's place, and what was read stays read.
    pre.scrollTop = commandScroll.get(id) ?? 0;
    pre.addEventListener("scroll", () => commandScroll.set(id, pre.scrollTop), { passive: true });
    if (!commandRead.has(id)) lockUntilRead(pre, allow, unread, () => commandRead.add(id));
    else unread.hidden = true;
  }
  noticeEl.onclick = n.kind === "approval" ? null : () => dismiss();
}

function drawOverview() {
  $("activity-label").textContent = activityLabel(activityState(), agentName);
  $("pin-tools").setAttribute("aria-pressed", String(pinned));
  $("pin-tools").title = pinned ? "Dejar de mantener abierto" : "Mantener abierto al retirar el cursor";
  if (tools.tab === "home") {
    drawMusic();
    drawAgents();
    drawFocus();
    drawShortcuts();
  }
  drawUsage();
  if (youtube.state.viewer && youtube.state.destination === "notch") $("usage").hidden = true;
}

/** One mark and percentage per provider, with the window and reset in the tooltip. */
function drawUsage() {
  const el = $("usage");
  if (tools.tab === "files") {
    el.hidden = true;
    el.replaceChildren();
    return;
  }
  const plans = usage.flatMap((plan) => {
    const window = compactWindow(plan);
    if (!window) return [];
    const help = usageHelp(plan, window);
    const level = window.usedPct >= 90 ? "high" : window.usedPct >= 70 ? "warn" : "";
    const mark = providerMark(plan.provider, 12);
    mark.removeAttribute("title");
    return [h("span", { class: `usage-chip ${level}`, title: help, "aria-label": help, tabindex: "0" },
      mark, h("span", { text: `${Math.round(window.usedPct)} %` }))];
  });
  el.hidden = !plans.length;
  el.replaceChildren(...plans);
}

function drawMusic() {
  const music = $("music");
  if (youtube.state.detected) { music.hidden = true; return; }
  music.hidden = !track;
  if (!track) return;
  const t = track;
  const playing = t.status === "playing";
  const control = (path: string, title: string, action: string, size: number) =>
    h("button", { class: "icon-btn", type: "button", title, "aria-label": title, onclick: () => void invoke("media_control", { action }) }, icon(path, size));
  const info: Node[] = [h("strong", { text: t.title }), h("span", { class: "muted", text: [t.artist, t.app].filter(Boolean).join(" · ") })];
  if (t.durationMs && t.positionMs !== null) {
    const now = Math.max(0, Math.min(t.positionMs + (playing ? Date.now() - trackAt : 0), t.durationMs));
    const fill = h("div", { class: "bar-fill", style: `width:${(now / t.durationMs) * 100}%` });
    info.push(h("div", { class: "progress" }, h("span", { id: "track-elapsed", text: clock(now / 1000) }), h("div", { class: "bar-track" }, fill),
      h("span", { id: "track-remaining", text: `-${clock((t.durationMs - now) / 1000)}` })));
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
    rows.push(h("span", { class: "muted", text: hooksConnected ? "Sin sesiones abiertas" : "Claude, Codex y Gemini" }));
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
    for (const [stroke, dash] of [["rgba(255,255,255,0.15)", c], ["#7c8cff", (left() / total) * c]] as const) {
      const circle = document.createElementNS(ns, "circle");
      const attrs = { cx: "23", cy: "23", r: String(r), fill: "none", stroke, "stroke-width": "4", "stroke-linecap": "round", "stroke-dasharray": `${dash} ${c}` };
      for (const [k, v] of Object.entries(attrs)) circle.setAttribute(k, v);
      ring.append(circle);
    }
    body.push(h("div", { class: "row-btns" },
      h("div", { class: "ring" }, ring, h("span", { text: clock(left()) })),
      h("button", { class: "pill", type: "button", title: "Terminar el bloque de enfoque ahora", onclick: () => void invoke("focus_stop") }, "Parar")));
  } else {
    const duration = h("select", { class: "focus-duration", "aria-label": "Elegir duración del enfoque", title: "Bloques de enfoque de 5 a 120 minutos" },
      h("option", { value: "", text: "Elegir duración" }),
      ...[5, 15, 25, 50, 90, 120].map((minutes) => h("option", { value: minutes, text: `${minutes} minutos` })));
    duration.addEventListener("change", () => {
      const minutes = Number(duration.value);
      if (minutes) void invoke("focus_start", { minutes });
      duration.value = "";
    });
    body.push(duration,
      h("div", { class: "row-btns" },
        h("button", { class: "pill primary", type: "button", title: "Empezar 25 minutos de enfoque", onclick: () => void invoke("focus_start", { minutes: 25 }) }, "25 min"),
        h("button", { class: "pill", type: "button", title: "Empezar 50 minutos de enfoque", onclick: () => void invoke("focus_start", { minutes: 50 }) }, "50")));
  }
  $("focus").replaceChildren(h("h2", {}, icon(TABLER.history, 14), "Enfoque"), ...body);
}

/** Only update time and progress: keep buttons and keyboard focus intact between ticks. */
function updateTimers() {
  if (focus?.running) {
    const ring = document.querySelector("#focus .ring");
    const label = ring?.querySelector("span");
    if (label) label.textContent = clock(left());
    const remaining = ring?.querySelector("circle:last-child");
    const c = 2 * Math.PI * 20;
    remaining?.setAttribute("stroke-dasharray", `${Math.min(left() / Math.max(focus.endsAt - focus.startedAt, 1), 1) * c} ${c}`);
  }
  if (track?.durationMs && track.positionMs !== null) {
    const now = Math.max(0, Math.min(track.positionMs + (track.status === "playing" ? Date.now() - trackAt : 0), track.durationMs));
    const elapsed = document.getElementById("track-elapsed");
    const remaining = document.getElementById("track-remaining");
    const fill = document.querySelector<HTMLElement>("#music .bar-fill");
    if (elapsed) elapsed.textContent = clock(now / 1000);
    if (remaining) remaining.textContent = `-${clock((track.durationMs - now) / 1000)}`;
    if (fill) fill.style.width = `${now / track.durationMs * 100}%`;
  }
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
  dropEl.replaceChildren(h("div", { class: "drop-target" }, icon(TABLER.folder, 22), h("span", { text: "Suelta tus archivos aquí" })));
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
  // A question stays long enough to be read.
  if (n.kind !== "approval") dismissTimer = window.setTimeout(() => { if (!hovering) dismiss(); }, (n.question ? 20 : NOTICE_SECONDS) * 1000);
  render();
}

function dismiss() {
  window.clearTimeout(dismissTimer);
  notice = null;
  const next = queue.shift();
  if (next) present(next);
  else render();
}

/** «Permitir siempre»: this one and the next ones with the same program and subcommand. */
function answerAlways(requestId: string) {
  void invoke("answer_approval_always", { requestId });
  closeApproval(requestId);
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
    case "youTubeChanged":
      void youtube.refresh();
      break;
    case "approvalRequest": {
      const a = e as Extract<CoreEvent, { type: "approvalRequest" }>;
      pendingApproval.set(a.requestId, a.sessionId);
      show({ kind: "approval", agent: a.agent, requestId: a.requestId, canAllow: a.canAllow, always: a.always,
        title: a.agent === "buddy" ? `Buddy quiere ${a.title.charAt(0).toLowerCase()}${a.title.slice(1)}` : `${agentName(a.agent)} pide permiso en ${a.project}`,
        detail: `${a.title}: ${a.summary}`, command: a.detail });
      break;
    }
    case "approvalClosed": {
      const id = (e as Extract<CoreEvent, { type: "approvalClosed" }>).requestId;
      pendingApproval.delete(id);
      commandScroll.delete(id);
      commandRead.delete(id);
      closeApproval(id);
      break;
    }
    case "usageChanged":
      void invoke<ProviderUsage[]>("usage").then((u) => { usage = u; if (mode() === "open") render(); });
      break;
    case "usageLow": {
      const u = e as Extract<CoreEvent, { type: "usageLow" }>;
      const name = planName(u.provider);
      show({ kind: "waiting", agent: u.provider, title: `Te queda ${u.leftPct} % de ${name}`, detail: `Ventana: ${u.label}. Buddy usará el otro proveedor si se acaba.` });
      break;
    }
    case "financeRecorded": {
      const f = e as Extract<CoreEvent, { type: "financeRecorded" }>;
      const cap = (t: string) => t.charAt(0).toUpperCase() + t.slice(1);
      show({ kind: "finished", agent: "niko", title: `Niko anotó: ${f.monto} · ${f.comercio || f.concepto}`,
        detail: f.concepto && f.comercio ? `${cap(f.tipo)} · ${f.concepto}` : cap(f.tipo) });
      break;
    }
    case "budgetAlert": {
      const b = e as Extract<CoreEvent, { type: "budgetAlert" }>;
      const name = b.categoria.charAt(0).toUpperCase() + b.categoria.slice(1);
      show({ kind: "waiting", agent: "niko",
        title: b.usadoPct >= 100 ? `Te pasaste del presupuesto de ${name}` : `Te queda ${100 - b.usadoPct} % en ${name}`,
        detail: `Llevas ${b.usadoPct} % del tope del mes. Pregúntale a Niko en qué se fue.` });
      break;
    }
    case "mascotState": {
      const state = (e as Extract<CoreEvent, { type: "mascotState" }>).state;
      const busy = state === "think" || state === "work";
      if (!busy) buddyActivity = null;
      if (busy !== buddyBusy) { buddyBusy = busy; render(); }
      break;
    }
    case "chatActivity": {
      const a = e as unknown as { kind: string; label: string };
      buddyActivity = { kind: a.kind, label: a.label };
      render();
      break;
    }
    case "chatDone":
    case "chatFailed":
      buddyActivity = null;
      render();
      break;
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
              // A question the agent asked (Codex's question tool) comes with its text and options.
              show({ kind: "waiting", agent: s.agent, title: s.summary ? `${name} te pregunta en ${s.project}` : `${name} espera tu respuesta`, detail: s.summary || s.project, question: !!s.summary });
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
  window.clearTimeout(leaveTimer);
  window.clearTimeout(dismissTimer);
  if (hovering || hoverTimer) return;
  hoverTimer = window.setTimeout(() => {
    hoverTimer = 0;
    hovering = true;
    if (!notice) {
      void refreshHooks();
      void invoke("refresh_usage");
    }
    render();
  }, 120);
});
island.addEventListener("mouseleave", () => {
  window.clearTimeout(hoverTimer);
  hoverTimer = 0;
  window.clearTimeout(leaveTimer);
  leaveTimer = window.setTimeout(() => {
    hovering = false;
    collapsedByUser = false;
    if (notice && notice.kind !== "approval") dismissTimer = window.setTimeout(dismiss, 2000);
    render();
  }, 300);
});

$("pin-tools").append(icon("M16 3l5 5l-4 1l-3 6l-5 -5l6 -3z M3 21l6 -6", 14));
$("close-tools").append(icon(TABLER.x, 14));
$("pin-tools").addEventListener("click", () => { pinned = !pinned; render(); });
function closeTools() {
  if (youtube.state.destination === "notch") youtube.close();
  pinned = false;
  collapsedByUser = true;
  if (notice && notice.kind !== "approval") dismiss();
  else render();
}
$("close-tools").addEventListener("click", closeTools);
document.addEventListener("keydown", (event) => {
  if (event.key === "Escape") { event.preventDefault(); closeTools(); }
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
    hovering = true;
    collapsedByUser = false;
    tools.addFiles(payload.paths);
    render();
  }
});

void listen<CoreEvent>("core-event", ({ payload }) => onCore(payload));
void youtube.refresh();
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
import "../shortcuts";
