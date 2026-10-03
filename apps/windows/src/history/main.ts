// The history search (Windows), in the centre of the screen: type to filter (the core matches every word in titles
// and messages, ignoring accents), arrows to move, Enter to open, Esc to close. Ctrl/Shift-click or «Seleccionar» pick
// several chats; Supr (or the trash on a row) deletes them, always after asking. Twin of Sources/Chat/History.swift.
import { invoke } from "@tauri-apps/api/core";
import { emitTo } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { applyTokens } from "../tokens";
import { h, svg } from "../chat/dom";
import { TABLER } from "../chat/tabler";

applyTokens();

interface ChatSummary { id: string; title: string; updatedAt: number; preview: string }

const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;
const query = $<HTMLInputElement>("query");
const results = $("results");
const selectButton = $<HTMLButtonElement>("select");
const bar = $("bar");
const count = $("count");
const confirmDialog = $<HTMLDialogElement>("confirm");
const icon = (path: string, size = 16) =>
  svg(path, size, { fill: "none", stroke: "currentColor", "stroke-width": "1.75", "stroke-linecap": "round", "stroke-linejoin": "round" });
$("glass").append(icon(TABLER.search, 18));

let chats: ChatSummary[] = [];
/** The highlighted row (pointer or arrows). */
let selected = 0;
/** Chats picked to be deleted together. */
const marked = new Set<string>();
/** Where a Shift-click range starts. */
let anchor: string | null = null;
/** «Seleccionar» is on: every row shows its checkbox and a click marks it instead of opening it. */
let selecting = false;

const picking = () => selecting || marked.size > 0;

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
  const pick = picking();
  results.classList.toggle("picking", pick);
  results.setAttribute("aria-multiselectable", String(pick));
  selectButton.hidden = !selecting && !chats.length;
  selectButton.textContent = selecting ? "Listo" : "Seleccionar";
  selectButton.title = selecting ? "Dejar de seleccionar" : "Elegir varios chats para borrarlos";
  selectButton.setAttribute("aria-pressed", String(selecting));
  bar.hidden = marked.size === 0;
  count.textContent = marked.size === 1 ? "1 seleccionado" : `${marked.size} seleccionados`;
  $("delete").title = marked.size === 1 ? "Borrar el chat seleccionado (Supr)" : "Borrar los chats seleccionados (Supr)";
  query.removeAttribute("aria-activedescendant");

  if (!chats.length) {
    results.append(h("div", { class: "empty-state", text: query.value.trim() ? `Nada coincide con «${query.value.trim()}»` : "Sin chats todavía" }));
    return;
  }
  let lastDay = -1;
  chats.forEach((c, i) => {
    const day = dayOf(c.updatedAt);
    if (day !== lastDay) {
      results.append(h("div", { class: "day", role: "presentation", text: DAYS[day]! }));
      lastDay = day;
    }
    const isMarked = marked.has(c.id);
    const trash = h("button", { class: "trash", type: "button", title: "Borrar chat", "aria-label": `Borrar «${c.title}»`, tabindex: -1 },
      icon(TABLER.trash));
    trash.addEventListener("click", (e) => { e.stopPropagation(); void confirmDelete([c.id]); });
    const row = h("div", {
      id: `chat-${i}`,
      class: `row${i === selected ? " selected" : ""}${isMarked ? " marked" : ""}`,
      role: "option",
      "aria-selected": pick ? String(isMarked) : String(i === selected),
      title: c.title,
    },
      pick ? h("span", { class: "check", "aria-hidden": "true" }, isMarked ? icon(TABLER.check, 12) : null) : null,
      h("strong", { text: c.title }),
      h("small", { text: label(c.updatedAt) }),
      c.preview ? h("span", { class: "preview", text: c.preview }) : null,
      pick ? null : trash);
    row.addEventListener("click", (e) => click(c.id, e));
    row.addEventListener("mousemove", () => { if (selected !== i) { selected = i; render(); } });
    row.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      void confirmDelete(isMarked && marked.size > 1 ? [...marked] : [c.id]);
    });
    results.append(row);
    if (i === selected) query.setAttribute("aria-activedescendant", row.id);
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

// MARK: Selecting

/** A click opens the chat; Ctrl-click marks or unmarks it, Shift-click marks the range from the last one. */
function click(id: string, e: MouseEvent) {
  if (e.shiftKey) extend(id);
  else if (e.ctrlKey || e.metaKey || picking()) toggle(id);
  else { void open(id); return; }
  render();
}

function toggle(id: string) {
  if (!marked.delete(id)) marked.add(id);
  anchor = id;
}

function extend(id: string) {
  const ids = chats.map((c) => c.id);
  const end = ids.indexOf(id);
  if (end < 0) return;
  const from = anchor ? ids.indexOf(anchor) : -1;
  const start = from < 0 ? end : from;
  for (const each of ids.slice(Math.min(start, end), Math.max(start, end) + 1)) marked.add(each);
  anchor ??= id;
}

function clearSelection() {
  marked.clear();
  anchor = null;
  selecting = false;
  render();
}

selectButton.addEventListener("click", () => {
  if (selecting) clearSelection();
  else { selecting = true; render(); }
  query.focus();
});
$("cancel").addEventListener("click", () => { clearSelection(); query.focus(); });
$("delete").addEventListener("click", () => void confirmDelete([...marked]));

// MARK: Deleting

/** What Supr deletes: the marked chats, or else the highlighted one. */
function deleteRequested() {
  if (marked.size) void confirmDelete([...marked]);
  else if (chats[selected]) void confirmDelete([chats[selected]!.id]);
}

/** Always asks first; nothing is deleted without «Borrar». */
async function confirmDelete(ids: string[]) {
  if (!ids.length || confirmDialog.open) return;
  const title = ids.length === 1 ? chats.find((c) => c.id === ids[0])?.title : undefined;
  $("confirm-title").textContent = ids.length === 1 ? "¿Borrar 1 chat?" : `¿Borrar ${ids.length} chats?`;
  $("confirm-text").textContent = title ? `«${title}». No se puede deshacer.` : "No se puede deshacer.";
  confirmDialog.returnValue = "";
  confirmDialog.showModal();
  const answer = await new Promise<string>((resolve) =>
    confirmDialog.addEventListener("close", () => resolve(confirmDialog.returnValue), { once: true }));
  query.focus();
  if (answer === "delete") await remove(ids);
}

/** Deletes in the core, tells the chat window (it starts over if it was showing one of them) and reloads the list. */
async function remove(ids: string[]) {
  const gone: string[] = [];
  for (const id of ids) {
    try {
      await invoke("delete_chat", { chatId: id });
      gone.push(id);
    } catch (error) {
      console.error("delete_chat", error);
    }
  }
  for (const id of gone) marked.delete(id);
  if (anchor && gone.includes(anchor)) anchor = null;
  if (gone.length) void emitTo("chat", "chats-deleted", gone).catch(() => {});
  await search();
  if (!chats.length) clearSelection();
}

// MARK: Keys

query.addEventListener("input", () => { selected = 0; void search(); });
window.addEventListener("keydown", (e) => {
  if (confirmDialog.open) return; // The dialog has its own keys (Esc cancels, Tab between its buttons).
  if (e.key === "Escape") {
    if (picking()) clearSelection();
    else void getCurrentWindow().hide();
  } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    selected = Math.min(Math.max(selected + (e.key === "ArrowDown" ? 1 : -1), 0), chats.length - 1);
    render();
  } else if (e.key === "Enter" && chats[selected]) {
    if (picking()) { toggle(chats[selected]!.id); render(); }
    else void open(chats[selected]!.id);
  } else if (e.key === "Delete" || e.key === "Backspace") {
    // Only when there is no search text for the key to erase.
    const atEnd = query.selectionStart === query.value.length && query.selectionEnd === query.value.length;
    const free = e.key === "Delete" ? atEnd : !query.value;
    if (!free || !chats.length || e.repeat) return;
    e.preventDefault();
    deleteRequested();
  }
});
void getCurrentWindow().onFocusChanged(({ payload: focused }) => {
  if (focused) { query.select(); void search(); } else if (!confirmDialog.open) void getCurrentWindow().hide();
});
void search();
query.focus();
import "../shortcuts";
