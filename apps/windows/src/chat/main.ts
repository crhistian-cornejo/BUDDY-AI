// The chat window next to Buddy (Windows): the composer at the bottom and, once there is an answer, the messages above
// it. The core does the work; this draws its events. Twin of Sources/Chat on the Mac.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { applyTokens } from "../tokens";
import { AnswerView } from "./answer";
import { h, svg } from "./dom";
import { TABLER } from "./tabler";
import { makeSource, type ChatSource } from "./markdown";

applyTokens();

interface ChatSummary { id: string; title: string; updatedAt: number; preview: string }
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
const recent = $("recent");
const input = $<HTMLTextAreaElement>("input");
const send = $<HTMLButtonElement>("send");

let chatId: string | null = null;
let streaming = false;
/** The answer being written. */
let live: { view: AnswerView; author: HTMLElement; activity: HTMLElement; text: string; sources: ChatSource[] } | null = null;

const icon = (path: string, size = 16) => svg(path, size, { fill: "none", stroke: "currentColor", "stroke-width": "1.75", "stroke-linecap": "round", "stroke-linejoin": "round" });
$("new").append(icon(TABLER.edit));
$("history").append(icon(TABLER.history));
$("close").append(icon(TABLER.x));

const PROVIDERS: Record<string, string> = { claude: "Claude", codex: "Codex", antigravity: "Gemini" };
const author = (name: string, provider?: string | null) => (provider ? `${name} · ${PROVIDERS[provider] ?? provider}` : name);

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

function setAuthor(el: HTMLElement, name: string) {
  el.replaceChildren(icon(name.startsWith("Buddy") ? TABLER.sparkles : TABLER.gitBranch, 14), name);
}

function setActivity(el: HTMLElement, activity: Activity | null) {
  el.hidden = !activity;
  if (activity) el.replaceChildren(icon(activity.icon, 16), h("span", { text: activity.text }));
}

function failure(text: string) {
  return h("div", { class: "msg-failed" }, icon(TABLER.alertTriangle, 16), h("span", { text }));
}

function addAnswer(name: string, text = "", sources: ChatSource[] = [], failed = false) {
  const authorEl = h("div", { class: "msg-author" });
  setAuthor(authorEl, name);
  const activityEl = h("div", { class: "activity", role: "status" });
  activityEl.hidden = true;
  const wrap = h("div", { class: "msg-assistant" }, authorEl, activityEl);
  const view = new AnswerView();
  if (failed) wrap.append(failure(text));
  else {
    view.update(text, { sources });
    wrap.append(view.el);
  }
  list.append(wrap);
  return { view, author: authorEl, activity: activityEl };
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
  const { view, author: a, activity } = addAnswer("Buddy");
  live = { view, author: a, activity, text: "", sources: [] };
  setActivity(activity, THINKING);
  streaming = true;
  render();
  try {
    chatId = await invoke<string>("send_message", { chatId, text });
  } catch (e) {
    finish(`No se pudo enviar: ${e}`);
  }
}

function finish(failureText: string | null) {
  if (live) {
    setActivity(live.activity, null);
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
      setAuthor(live.author, author(e.agentName, e.provider));
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
  recent.hidden = true;
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
      addAnswer(author(names.get(m.agent) ?? m.agent, m.provider), m.text, sources, m.failed);
    }
  }
  render();
}

async function toggleRecent() {
  if (!recent.hidden) {
    recent.hidden = true;
    return;
  }
  const chats = await invoke<ChatSummary[]>("chats", { limit: 12 });
  recent.textContent = "";
  if (!chats.length) recent.append(h("div", { class: "empty", text: "Sin chats todavía" }));
  for (const c of chats) recent.append(h("button", { type: "button", text: c.title, title: c.preview, onclick: () => void openChat(c.id) }));
  recent.hidden = false;
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
$("history").addEventListener("click", () => void toggleRecent());
$("close").addEventListener("click", close);
window.addEventListener("keydown", (e) => {
  if (e.key === "Escape") close();
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
render();
input.focus();
