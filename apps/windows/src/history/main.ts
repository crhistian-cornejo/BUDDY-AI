// The history search (Windows), in the centre of the screen: type to filter (the core matches every word in titles
// and messages, ignoring accents), arrows to move, Enter to open, Esc to close. Twin of Sources/Chat/History.swift.
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { applyTokens } from "../tokens";
import { h, svg } from "../chat/dom";
import { TABLER } from "../chat/tabler";

applyTokens();

interface ChatSummary { id: string; title: string; updatedAt: number; preview: string }

const query = document.getElementById("query") as HTMLInputElement;
const results = document.getElementById("results")!;
document.getElementById("glass")!.append(svg(TABLER.search, 18, { fill: "none", stroke: "currentColor", "stroke-width": "1.75", "stroke-linecap": "round", "stroke-linejoin": "round" }));

let chats: ChatSummary[] = [];
let selected = 0;

const DAYS = ["Hoy", "Ayer", "Esta semana", "Este mes", "Anteriores"];

function dayOf(seconds: number, now = new Date()): number {
  const start = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const days = Math.round((start(now) - start(new Date(seconds * 1000))) / 86_400_000);
  return days < 1 ? 0 : days === 1 ? 1 : days < 7 ? 2 : days < 30 ? 3 : 4;
}

function label(seconds: number): string {
  const date = new Date(seconds * 1000);
  switch (dayOf(seconds)) {
    case 0: return date.toLocaleTimeString("es", { hour: "2-digit", minute: "2-digit" });
    case 1: return "ayer";
    case 2: return date.toLocaleDateString("es", { weekday: "long" });
    default: return date.toLocaleDateString("es", { day: "numeric", month: "short" }).replace(".", "");
  }
}

function render() {
  results.textContent = "";
  if (!chats.length) {
    results.append(h("div", { class: "empty-state", text: query.value.trim() ? `Nada coincide con «${query.value.trim()}»` : "Sin chats todavía" }));
    return;
  }
  let lastDay = -1;
  chats.forEach((c, i) => {
    const day = dayOf(c.updatedAt);
    if (day !== lastDay) {
      results.append(h("div", { class: "day", text: DAYS[day]! }));
      lastDay = day;
    }
    const row = h("button", { class: `row${i === selected ? " selected" : ""}`, type: "button", role: "option", title: c.title },
      h("strong", { text: c.title }), h("small", { text: label(c.updatedAt) }), c.preview ? h("span", { text: c.preview }) : null);
    row.addEventListener("click", () => void open(c.id));
    row.addEventListener("mousemove", () => { if (selected !== i) { selected = i; render(); } });
    row.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      if (confirm(`¿Eliminar el chat «${c.title}»?`)) void invoke("delete_chat", { chatId: c.id }).then(search);
    });
    results.append(row);
  });
  results.querySelector(".row.selected")?.scrollIntoView({ block: "nearest" });
}

async function search() {
  chats = await invoke<ChatSummary[]>("search_chats", { query: query.value, limit: 60 });
  selected = Math.min(selected, Math.max(chats.length - 1, 0));
  render();
}

async function open(id: string) {
  await invoke("history_pick", { chatId: id });
}

query.addEventListener("input", () => { selected = 0; void search(); });
window.addEventListener("keydown", (e) => {
  if (e.key === "Escape") void getCurrentWindow().hide();
  else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    selected = Math.min(Math.max(selected + (e.key === "ArrowDown" ? 1 : -1), 0), chats.length - 1);
    render();
  } else if (e.key === "Enter" && chats[selected]) void open(chats[selected]!.id);
});
void getCurrentWindow().onFocusChanged(({ payload: focused }) => {
  if (focused) { query.select(); void search(); } else void getCurrentWindow().hide();
});
void search();
query.focus();
import "../shortcuts";
