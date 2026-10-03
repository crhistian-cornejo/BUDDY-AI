// The chat window next to Buddy (Windows): the composer at the bottom and, once there is an answer, the messages above
// it. The core does the work; this draws its events. Twin of Sources/Chat on the Mac.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { applyTokens } from "../tokens";
import { AnswerView } from "./answer";
import { h, svg } from "./dom";
import { TABLER } from "./tabler";
import { faceForName } from "./avatar";
import { PROVIDER_MARKS } from "./provider-marks";
import { makeSource, type ChatSource } from "./markdown";

applyTokens();

interface SavedMessage { id: number; role: string; agent: string; provider?: string | null; text: string; sources: { title: string; url: string }[]; failed: boolean; attachments?: string[] }
interface QueuedMessage { id: string; text: string; attachments: string[] }
interface Agent { id: string; name: string }
type CoreEvent =
  | { type: "chatDequeued"; chatId: string; text: string; attachments: string[] }
  | { type: "chatQueueChanged"; chatId: string }
  | { type: "chatStarted"; chatId: string; agent: string; agentName: string; provider: string }
  | { type: "chatDelta"; chatId: string; text: string }
  | { type: "chatTool"; chatId: string; name: string; summary: string }
  | { type: "chatSource"; chatId: string; title: string; url: string }
  | { type: "chatDone"; chatId: string; messageId: number }
  | { type: "chatFailed"; chatId: string; message: string }
  | { type: string; chatId?: string };

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const panel = $("panel");
const list = $("messages");
const input = $<HTMLTextAreaElement>("input");
const send = $<HTMLButtonElement>("send");

let chatId: string | null = null;
let submitting = false;
let chatGeneration = 0;
let submissionGeneration = -1;
let queueRevision = 0;
let queued: QueuedMessage[] = [];
/** Files waiting to go with the next message. */
let files: string[] = [];
const filesEl = $("files");
const baseName = (p: string) => p.split(/[\\/]/).pop() ?? p;

function chip(path: string, onRemove?: () => void): HTMLElement {
  const el = h("span", { class: "chip", title: path }, icon(TABLER.fileText, 14), h("span", { text: baseName(path) }));
  if (onRemove) el.append(h("button", { type: "button", title: "Quitar", "aria-label": "Quitar", onclick: onRemove }, icon(TABLER.x, 12)));
  return el;
}

function drawFiles() {
  filesEl.hidden = !files.length;
  filesEl.replaceChildren(...files.map((f) => chip(f, () => { files = files.filter((x) => x !== f); drawFiles(); render(); })));
}

function attach(paths: string[]) {
  for (const p of paths) if (!files.includes(p)) files.push(p);
  drawFiles();
  render();
}
let streaming = false;
/** The answer being written. */
let live: { view: AnswerView; author: HTMLElement; activity: HTMLElement; actions: HTMLElement; text: string; sources: ChatSource[] } | null = null;

const icon = (path: string, size = 16) => svg(path, size, { fill: "none", stroke: "currentColor", "stroke-width": "1.75", "stroke-linecap": "round", "stroke-linejoin": "round" });
$("new").append(icon(TABLER.edit));
$("attach").append(icon(TABLER.paperclip));
$("history").append(icon(TABLER.history));
$("close").append(icon(TABLER.x));
$("stop").append(icon(TABLER.playerStop));
$("stop").addEventListener("click", () => { if (chatId) void invoke("cancel_chat", { chatId }); });

function queueError(text: string | null) {
  $("queue-error").textContent = text;
  $("queue-error").hidden = !text;
}

async function editQueued(item: QueuedMessage) {
  if (!chatId) return;
  const id = chatId;
  try {
    const editing = await invoke<QueuedMessage>("take_queued", { chatId: id, messageId: item.id });
    if (id !== chatId) return;
    input.value = input.value ? `${editing.text}\n${input.value}` : editing.text;
    attach(editing.attachments);
    autosize(); input.focus(); queueError(null);
    await refreshQueue();
  } catch (e) { queueError(`No se pudo editar: ${e}`); }
}

const thumbnails = new Map<string, Promise<string | null>>();
function queueAttachment(item: QueuedMessage) {
  const path = item.attachments[0]!;
  const el = h("span", { class: "queue-thumb", title: baseName(path) }, icon(TABLER.fileText, 16));
  if (/\.(png|jpe?g|gif|webp)$/i.test(path) && chatId) {
    let preview = thumbnails.get(item.id);
    if (!preview) {
      preview = invoke<string | null>("queued_thumbnail", { chatId, messageId: item.id }).catch(() => null);
      thumbnails.set(item.id, preview);
    }
    void preview.then((src) => {
      if (src) el.replaceChildren(h("img", { src, alt: baseName(path) }));
    });
  }
  return el;
}

function drawQueue() {
  const el = $("queue");
  el.hidden = !queued.length;
  el.replaceChildren(h("div", { class: "queue-list" }, ...queued.map((item) => {
    const menu = h("details", { class: "queue-menu" },
      h("summary", { title: "Más opciones", "aria-label": "Más opciones" }, icon(TABLER.dots, 16)),
      h("div", { class: "queue-options" },
        h("button", { type: "button", text: "Editar mensaje", onclick: () => { menu.open = false; void editQueued(item); } }),
        h("button", { type: "button", text: "Copiar mensaje", onclick: () => { menu.open = false; void navigator.clipboard.writeText(item.text); } })));
    return h("div", { class: "queue-item" },
      icon(TABLER.queue, 14),
      ...(item.attachments.length ? [queueAttachment(item)] : []),
      h("span", { class: "queue-text", text: item.text, title: item.text }),
      h("button", { type: "button", class: "queue-redirect", title: "Detener la respuesta actual y enviar este mensaje", onclick: () => {
        if (chatId) void invoke("redirect_queued", { chatId, messageId: item.id }).catch((e) => queueError(`No se pudo redirigir: ${e}`));
      } }, icon(TABLER.redirect, 14), "Redirigir"),
      h("button", { type: "button", class: "ghost small", title: "Eliminar de la cola", "aria-label": "Eliminar de la cola", onclick: () => {
        if (chatId) void invoke("remove_queued", { chatId, messageId: item.id });
      } }, icon(TABLER.trash, 14)), menu);
  })));
}

async function refreshQueue() {
  const revision = ++queueRevision;
  const id = chatId;
  if (!id) { queued = []; drawQueue(); return; }
  const pending = await invoke<QueuedMessage[]>("queued_messages", { chatId: id });
  if (id === chatId && revision === queueRevision) {
    queued = pending;
    for (const key of thumbnails.keys()) if (!pending.some((item) => item.id === key)) thumbnails.delete(key);
    drawQueue();
  }
}



interface Activity { icon: string; text: string }
/** No icon: only the shimmering text. */
const THINKING: Activity = { icon: "", text: "Pensando…" };

function activityFor(tool: string, summary: string): Activity {
  if (tool === "WebSearch") return { icon: TABLER.search, text: summary ? `Buscando: ${summary}` : "Buscando en la web…" };
  if (tool === "WebFetch") return { icon: TABLER.search, text: "Leyendo una página…" };
  if (tool === "Cambio") return { icon: TABLER.arrowsExchange, text: summary };
  return { icon: TABLER.dots, text: "Trabajando…" };
}

function addUser(text: string, attached: string[] = []) {
  if (attached.length) list.append(h("div", { class: "msg-files" }, ...attached.map((f) => chip(f))));
  list.append(h("div", { class: "msg-user", text }));
}

/** Buddy's face and the agent's name, with tooltips. */
function setAuthor(el: HTMLElement, name: string) {
  const face = h("img", { class: "avatar", alt: "", title: name === "Buddy" ? "Buddy" : `${name}, del equipo de Buddy` });
  void faceForName(name).then((url) => { if (url) face.setAttribute("src", url); });
  el.replaceChildren(face, h("span", { text: name }));
}

/** The mark of the service that wrote the answer, after the copy and redo buttons. */
function setMark(actions: HTMLElement, provider?: string | null) {
  actions.querySelector(".provider")?.remove();
  const mark = provider ? PROVIDER_MARKS[provider] : undefined;
  if (!mark) return;
  actions.append(h("span", { class: "provider", title: mark.label, "aria-label": mark.label, style: mark.color ? `color:${mark.color}` : "" },
    svg(mark.path, 12, { fill: "currentColor" })));
}

/** Copy (and, on the last answer, write again) under an answer. */
function actionsFor(getText: () => string): HTMLElement {
  const copy = h("button", { class: "ghost small", type: "button", title: "Copiar respuesta", "aria-label": "Copiar respuesta" }, icon(TABLER.copy, 14));
  copy.addEventListener("click", () => {
    void navigator.clipboard.writeText(getText());
    copy.replaceChildren(icon(TABLER.check, 14));
    copy.title = "Copiado";
    setTimeout(() => { copy.replaceChildren(icon(TABLER.copy, 14)); copy.title = "Copiar respuesta"; }, 1500);
  });
  const redo = h("button", { class: "ghost small redo", type: "button", title: "Rehacer la respuesta", "aria-label": "Rehacer la respuesta" }, icon(TABLER.refresh, 14));
  redo.addEventListener("click", () => void regenerate());
  return h("div", { class: "msg-actions" }, copy, redo);
}

function setActivity(el: HTMLElement, activity: Activity | null) {
  el.hidden = !activity;
  if (activity) el.replaceChildren(...(activity.icon ? [icon(activity.icon, 16)] : []), h("span", { class: "answer-status", text: activity.text }));
}

function failure(text: string) {
  return h("div", { class: "msg-failed" }, icon(TABLER.alertTriangle, 16), h("span", { text }));
}

const KINDS: Record<string, [string, string, string]> = {
  docx: ["W", "#2b579a", "Documento de Word"], doc: ["W", "#2b579a", "Documento de Word"],
  xlsx: ["X", "#217346", "Hoja de Excel"], xls: ["X", "#217346", "Hoja de Excel"],
  pptx: ["P", "#d24726", "Presentación de PowerPoint"], ppt: ["P", "#d24726", "Presentación de PowerPoint"],
  pdf: ["PDF", "#c62828", "PDF"],
};

/** A page with a folded corner, the app's colour band and its letter: our own drawing, no logos. */
function documentIcon(ext: string): SVGElement {
  const [letter, colour] = KINDS[ext] ?? ["", "#71717a"];
  const ns = "http://www.w3.org/2000/svg";
  const el = (tag: string, attrs: Record<string, string>) => {
    const node = document.createElementNS(ns, tag);
    for (const [k, v] of Object.entries(attrs)) node.setAttribute(k, v);
    return node;
  };
  const svgEl = el("svg", { viewBox: "0 0 32 32", width: "32", height: "32", "aria-hidden": "true" });
  svgEl.append(
    el("path", { d: "M7 3h13l6 6v20H7z", fill: "#fff", stroke: "rgba(0,0,0,.18)" }),
    el("path", { d: "M20 3v6h6", fill: "none", stroke: "rgba(0,0,0,.18)" }),
    el("path", { d: "M10 14h12M10 18h12M10 22h8", stroke: "rgba(0,0,0,.18)", "stroke-width": "1.5" }),
    el("rect", { x: "2", y: "15", width: letter.length > 1 ? "18" : "13", height: "13", rx: "2.5", fill: colour }),
  );
  const label = el("text", { x: letter.length > 1 ? "11" : "8.5", y: "24.6", "text-anchor": "middle", fill: "#fff", "font-size": letter.length > 1 ? "7" : "9", "font-weight": "700", "font-family": "Segoe UI, system-ui, sans-serif" });
  label.textContent = letter;
  svgEl.append(label);
  return svgEl;
}

/** A document an agent made: its icon, its name and kind; a click opens it in its app. */
function documentCard(path: string): HTMLElement {
  const name = path.split(/[\\/]/).pop() ?? path;
  const dot = name.lastIndexOf(".");
  const ext = dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
  const title = dot > 0 ? name.slice(0, dot) : name;
  const card = h("button", { class: "doc-card", type: "button", title: "Abrir", "aria-label": `${KINDS[ext]?.[2] ?? "Archivo"}: ${name}` },
    documentIcon(ext),
    h("span", { class: "doc-text" }, h("span", { class: "doc-name", text: title }), h("span", { class: "doc-kind", text: KINDS[ext]?.[2] ?? ext.toUpperCase() })),
    icon(TABLER.arrowUpRight, 14));
  card.addEventListener("click", () => void invoke("open_document", { path }).catch((e) => { card.title = `No se pudo abrir: ${e}`; }));
  return card;
}

function addAnswer(name: string, provider: string | null, text = "", sources: ChatSource[] = [], failed = false, documents: string[] = []) {
  const authorEl = h("div", { class: "msg-author" });
  setAuthor(authorEl, name);
  const activityEl = h("div", { class: "activity", role: "status" });
  activityEl.hidden = true;
  const wrap = h("div", { class: "msg-assistant" }, authorEl, activityEl);
  const view = new AnswerView();
  const state = { text };
  const actions = actionsFor(() => state.text);
  setMark(actions, provider);
  if (failed) wrap.append(failure(text));
  else {
    view.update(text, { sources });
    wrap.append(view.el);
  }
  actions.hidden = failed || !text;
  for (const d of documents) wrap.append(documentCard(d));
  wrap.append(actions);
  list.append(wrap);
  return { view, author: authorEl, activity: activityEl, actions, state };
}

function render() {
  panel.hidden = list.childElementCount === 0;
  send.replaceChildren(icon(streaming ? TABLER.arrowsExchange : TABLER.arrowUp, 16));
  send.title = streaming ? "Añadir a la cola (Enter)" : "Enviar (Enter)";
  send.setAttribute("aria-label", send.title);
  send.disabled = submitting || (!input.value.trim() && !files.length);
  $("stop").hidden = !streaming;
  drawQueue();
  const first = list.querySelector(".msg-user")?.textContent ?? "Buddy";
  $("title").textContent = first.split("\n")[0]!;
  list.scrollTop = list.scrollHeight;
}

async function submit() {
  if (submitting) return;
  const generation = chatGeneration;
  let text = input.value.trim();
  if (!text && !files.length) return;
  if (!text) text = files.length === 1 ? "Revisa este archivo." : "Revisa estos archivos.";
  const sending = files;
  files = [];
  drawFiles();
  input.value = "";
  autosize();
  submitting = true;
  submissionGeneration = generation;
  queueError(null);
  render();
  try {
    const id = await invoke<string>("send_message", { chatId, text, attachments: sending });
    if (generation === chatGeneration) { chatId = id; await refreshQueue(); }
  } catch (e) {
    if (generation === chatGeneration) {
      input.value = input.value ? `${text}\n${input.value}` : text;
      files = [...new Set([...sending, ...files])];
      drawFiles(); autosize();
      queueError(`No se pudo enviar: ${e}`);
    }
  } finally {
    submitting = false;
    render();
  }
}

/** A new answer being written (after a question, or to write the last one again). */
function startLive() {
  const { view, author: a, activity, actions, state } = addAnswer("Buddy", null);
  live = { view, author: a, activity, actions, text: "", sources: [] };
  liveState = state;
  setActivity(activity, THINKING);
}

let liveState: { text: string } = { text: "" };

async function regenerate() {
  if (streaming || !chatId) return;
  list.querySelectorAll(".msg-assistant").forEach((el, i, all) => { if (i === all.length - 1) el.remove(); });
  startLive();
  streaming = true;
  render();
  try {
    await invoke("regenerate", { chatId });
  } catch (e) {
    finish(`No se pudo rehacer: ${e}`);
  }
}

function finish(failureText: string | null) {
  // Documents the agent made in this turn come saved with the answer: show them as cards.
  if (live && !failureText && chatId) {
    const actionsEl = live.actions;
    void invoke<SavedMessage[]>("messages", { chatId }).then((saved) => {
      const last = [...saved].reverse().find((m) => m.role === "assistant");
      for (const d of last?.attachments ?? []) actionsEl.before(documentCard(d));
    }).catch(() => {});
  }
  if (live) {
    setActivity(live.activity, null);
    liveState.text = live.text;
    live.actions.hidden = !live.text;
    if (failureText && !live.text) {
      live.view.el.replaceWith(failure(failureText));
    } else {
      live.view.update(live.text, { sources: live.sources });
    }
  }
  live = null;
  streaming = false;
  render();
}

function onCore(event: CoreEvent) {
  if (event.type === "chatDequeued" && !chatId && submitting && submissionGeneration === chatGeneration) chatId = event.chatId ?? null;
  if (!("chatId" in event) || event.chatId !== chatId) return;
  if (event.type === "chatQueueChanged") { void refreshQueue(); return; }
  if (event.type === "chatDequeued") {
    const e = event as Extract<CoreEvent, { type: "chatDequeued" }>;
    addUser(e.text, e.attachments);
    startLive(); streaming = true;
    render(); void refreshQueue(); return;
  }
  if (!live) return;
  switch (event.type) {
    case "chatStarted": {
      const e = event as Extract<CoreEvent, { type: "chatStarted" }>;
      setAuthor(live.author, e.agentName);
      setMark(live.actions, e.provider);
      if (!live.text) {
        setActivity(live.activity, e.agent === "buddy" ? THINKING : { icon: TABLER.gitBranch, text: `Buddy le pasa la tarea a ${e.agentName}…` });
      }
      break;
    }
    case "chatDelta":
      live.text += (event as Extract<CoreEvent, { type: "chatDelta" }>).text;
      setActivity(live.activity, null);
      break;
    case "chatTool": {
      const e = event as Extract<CoreEvent, { type: "chatTool" }>;
      setActivity(live.activity, activityFor(e.name, e.summary));
      break;
    }
    case "chatSource": {
      const e = event as Extract<CoreEvent, { type: "chatSource" }>;
      const source = makeSource(e.title, e.url);
      if (source && !live.sources.some((s) => s.url === source.url)) live.sources.push(source);
      break;
    }
    case "chatDone":
      return finish(null);
    case "chatFailed":
      return finish((event as Extract<CoreEvent, { type: "chatFailed" }>).message);
    default:
      return;
  }
  live.view.update(live.text, { sources: live.sources });
  render();
}

async function openChat(id: string) {
  chatGeneration++;
  queueError(null);
  if (chatId) await invoke("cancel_chat", { chatId });
  const [messages, agents] = await Promise.all([
    invoke<SavedMessage[]>("messages", { chatId: id }),
    invoke<Agent[]>("agents"),
  ]);
  const names = new Map(agents.map((a) => [a.id, a.name]));
  chatId = id;
  void refreshQueue();
  streaming = false;
  live = null;
  list.textContent = "";
  for (const m of messages) {
    if (m.role === "user") addUser(m.text, m.attachments ?? []);
    else {
      const sources = m.sources.map((s) => makeSource(s.title, s.url)).filter((s): s is ChatSource => !!s);
      addAnswer(names.get(m.agent) ?? m.agent, m.provider ?? null, m.text, sources, m.failed, m.attachments ?? []);
    }
  }
  render();
}

function newChat() {
  chatGeneration++;
  queueError(null);
  queued = [];
  if (chatId) void invoke("cancel_chat", { chatId });
  chatId = null;
  streaming = false;
  live = null;
  list.textContent = "";
  render();
  input.focus();
}

function autosize() {
  input.style.height = "auto";
  input.style.height = `${Math.min(input.scrollHeight, 110)}px`;
}

const close = () => void invoke("close_chat");

$("composer").addEventListener("submit", (e) => {
  e.preventDefault();
  void submit();
});
input.addEventListener("keydown", (e) => {
  if (e.key === "Enter" && !e.shiftKey) {
    e.preventDefault();
    void submit();
  }
});
input.addEventListener("input", () => {
  autosize();
  render();
});
$("new").addEventListener("click", newChat);
$("history").addEventListener("click", () => void invoke("open_history"));
$("close").addEventListener("click", close);
window.addEventListener("keydown", (e) => {
  if (e.key === "Escape") close();
  else if (e.ctrlKey && e.key.toLowerCase() === "f") { e.preventDefault(); void invoke("open_history"); }
  else if (e.ctrlKey && e.key.toLowerCase() === "n") { e.preventDefault(); newChat(); }
});

// The window follows its content's height (bottom-anchored next to Buddy).
new ResizeObserver(() => {
  void invoke("chat_resize", { height: Math.ceil($("root").getBoundingClientRect().height) });
}).observe($("root"));

// Clicking elsewhere closes the chat, unless an answer is being written.
void getCurrentWindow().onFocusChanged(({ payload: focused }) => {
  if (focused) input.focus();
  else if (!streaming) close();
});

void listen<CoreEvent>("core-event", ({ payload }) => onCore(payload));
void listen<string>("open-chat", ({ payload }) => void openChat(payload));
// The history deleted chats: if the open one was among them, start over.
void listen<string[]>("chats-deleted", ({ payload }) => { if (chatId && payload.includes(chatId)) newChat(); });
// Files dropped on the top bar: a new chat with them attached.
void listen<string[]>("attach", ({ payload }) => {
  newChat();
  attach(payload);
  input.focus();
});
$("attach").addEventListener("click", () => void invoke<string[]>("pick_files").then(attach));
render();
input.focus();
import "../shortcuts";
