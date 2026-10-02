// The chat window next to Buddy (Windows): the composer at the bottom and, once there is an answer, the messages above
// it. The core does the work; this draws its events. Twin of Sources/Chat on the Mac.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { applyTokens } from "../tokens";
import { AnswerView } from "./answer";
import { h, svg } from "./dom";
import { TABLER } from "./tabler";
import { buddyFace } from "./avatar";
import { PROVIDER_MARKS } from "./provider-marks";
import { makeSource, type ChatSource } from "./markdown";

applyTokens();

interface SavedMessage { id: number; role: string; agent: string; provider?: string | null; text: string; sources: { title: string; url: string }[]; failed: boolean }
interface Agent { id: string; name: string }
type CoreEvent =
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
let streaming = false;
/** The answer being written. */
let live: { view: AnswerView; author: HTMLElement; activity: HTMLElement; actions: HTMLElement; text: string; sources: ChatSource[] } | null = null;

const icon = (path: string, size = 16) => svg(path, size, { fill: "none", stroke: "currentColor", "stroke-width": "1.75", "stroke-linecap": "round", "stroke-linejoin": "round" });
$("new").append(icon(TABLER.edit));
$("history").append(icon(TABLER.history));
$("close").append(icon(TABLER.x));


interface Activity { icon: string; text: string }
const THINKING: Activity = { icon: TABLER.dots, text: "Pensando…" };

function activityFor(tool: string, summary: string): Activity {
  if (tool === "WebSearch") return { icon: TABLER.search, text: summary ? `Buscando: ${summary}` : "Buscando en la web…" };
  if (tool === "WebFetch") return { icon: TABLER.search, text: "Leyendo una página…" };
  if (tool === "Cambio") return { icon: TABLER.arrowsExchange, text: summary };
  return { icon: TABLER.dots, text: "Trabajando…" };
}

function addUser(text: string) {
  list.append(h("div", { class: "msg-user", text }));
}

/** Buddy's face, the agent's name and the mark of the service that wrote the answer, with tooltips. */
function setAuthor(el: HTMLElement, name: string, provider?: string | null) {
  const face = h("img", { class: "avatar", alt: "", title: name === "Buddy" ? "Buddy" : `${name}, del equipo de Buddy` });
  void buddyFace().then((url) => { if (url) face.setAttribute("src", url); });
  const mark = provider ? PROVIDER_MARKS[provider] : undefined;
  const markEl = mark
    ? h("span", { class: "provider", title: mark.label, "aria-label": mark.label, style: mark.color ? `color:${mark.color}` : "" },
        svg(mark.path, 12, { fill: "currentColor" }))
    : null;
  el.replaceChildren(face, h("span", { text: name }), ...(markEl ? [markEl] : []));
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
  if (activity) el.replaceChildren(icon(activity.icon, 16), h("span", { text: activity.text }));
}

function failure(text: string) {
  return h("div", { class: "msg-failed" }, icon(TABLER.alertTriangle, 16), h("span", { text }));
}

function addAnswer(name: string, provider: string | null, text = "", sources: ChatSource[] = [], failed = false) {
  const authorEl = h("div", { class: "msg-author" });
  setAuthor(authorEl, name, provider);
  const activityEl = h("div", { class: "activity", role: "status" });
  activityEl.hidden = true;
  const wrap = h("div", { class: "msg-assistant" }, authorEl, activityEl);
  const view = new AnswerView();
  const state = { text };
  const actions = actionsFor(() => state.text);
  if (failed) wrap.append(failure(text));
  else {
    view.update(text, { sources });
    wrap.append(view.el);
  }
  actions.hidden = failed || !text;
  wrap.append(actions);
  list.append(wrap);
  return { view, author: authorEl, activity: activityEl, actions, state };
}

function render() {
  panel.hidden = list.childElementCount === 0;
  send.replaceChildren(icon(streaming ? TABLER.playerStop : TABLER.arrowUp, 16));
  send.title = streaming ? "Detener la respuesta" : "Enviar (Enter)";
  send.disabled = !streaming && !input.value.trim();
  const first = list.querySelector(".msg-user")?.textContent ?? "Buddy";
  $("title").textContent = first.split("\n")[0]!;
  list.scrollTop = list.scrollHeight;
}

async function submit() {
  if (streaming) {
    if (chatId) await invoke("cancel_chat", { chatId });
    return;
  }
  const text = input.value.trim();
  if (!text) return;
  input.value = "";
  autosize();
  addUser(text);
  startLive();
  streaming = true;
  render();
  try {
    chatId = await invoke<string>("send_message", { chatId, text });
  } catch (e) {
    finish(`No se pudo enviar: ${e}`);
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
  if (!live || !("chatId" in event) || event.chatId !== chatId) return;
  switch (event.type) {
    case "chatStarted": {
      const e = event as Extract<CoreEvent, { type: "chatStarted" }>;
      setAuthor(live.author, e.agentName, e.provider);
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
  if (streaming && chatId) await invoke("cancel_chat", { chatId });
  const [messages, agents] = await Promise.all([
    invoke<SavedMessage[]>("messages", { chatId: id }),
    invoke<Agent[]>("agents"),
  ]);
  const names = new Map(agents.map((a) => [a.id, a.name]));
  chatId = id;
  streaming = false;
  live = null;
  list.textContent = "";
  for (const m of messages) {
    if (m.role === "user") addUser(m.text);
    else {
      const sources = m.sources.map((s) => makeSource(s.title, s.url)).filter((s): s is ChatSource => !!s);
      addAnswer(names.get(m.agent) ?? m.agent, m.provider ?? null, m.text, sources, m.failed);
    }
  }
  render();
}

function newChat() {
  if (streaming && chatId) void invoke("cancel_chat", { chatId });
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
// Files dropped on the top bar: a new chat with them in the field.
void listen<string>("prefill", ({ payload }) => {
  newChat();
  input.value = payload;
  autosize();
  render();
  input.focus();
});
render();
input.focus();
