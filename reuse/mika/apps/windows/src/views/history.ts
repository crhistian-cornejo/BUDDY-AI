// History — every conversation with every agent, newest first and grouped by day, with a search box (twin of
// HistoryView.swift). A click opens it in its agent's chat; the conversation that was open there goes back to the
// History. Each agent's memory can be opened from here too. The grouping and the search are in core/history.ts.

import { h, svg, clear, dot } from "./dom";
import { ICONS } from "./icons";
import { Bridge, IS_TAURI, type HistoryEntry } from "../core/bridge";
import { Agents, agentTaskId } from "../core/agents";
import { createMiniBot } from "../character/minibots";
import { State } from "../core/state";
import { GROUP_TITLES, conversationsLabel, countLabel, groups, matching, rowTime, type HistoryGroup } from "../core/history";
import type { ViewActions, ViewHost } from "./views";

/** "hoy 14:05", "ayer", "28 sept." — in the user's own locale settings. */
export function whenLabel(ms: number, now = Date.now()): string {
  if (!ms) return "";
  const d = new Date(ms), today = new Date(now);
  const day = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const days = Math.round((day(today) - day(d)) / 86_400_000);
  if (days === 0) return `hoy ${d.toLocaleTimeString("es", { hour: "2-digit", minute: "2-digit" })}`;
  if (days === 1) return "ayer";
  if (days < 7) return d.toLocaleDateString("es", { weekday: "long" });
  return d.toLocaleDateString("es", { day: "numeric", month: "short", year: d.getFullYear() === today.getFullYear() ? undefined : "numeric" });
}

export function buildHistory(actions: ViewActions): ViewHost {
  let filter: string | null = null;
  let query = "";
  let entries: HistoryEntry[] = [];
  let error = "";
  /** The row whose delete button was pressed once: a second press deletes it. */
  let confirming = "";

  const heading = h("div", { class: "history-heading" }, h("span", { class: "history-name", text: "Historial" }));
  const count = h("span", { class: "history-count" });
  heading.append(count);
  const input = h("input", { class: "history-search-input", placeholder: "Buscar", "aria-label": "Buscar conversaciones", spellcheck: "false", autocomplete: "off" });
  input.type = "text";
  const clearSearch = h("button", { class: "history-search-clear", "aria-label": "Borrar la búsqueda", hidden: true }, svg(ICONS.xmark, 10));
  const search = h("div", { class: "history-search" }, svg(ICONS.search, 12), input, clearSearch);
  const memory = h("button", { class: "history-memory" }, h("span", { text: "Memoria" }));
  const chips = h("div", { class: "history-chips" });
  const list = h("div", { class: "history-list" });
  const el = h("div", { class: "view" }, h("div", { class: "card history-card" },
    h("div", { class: "history-top" }, heading, h("div", { class: "grow" }), search, memory), chips, list));

  input.addEventListener("input", () => { query = input.value; renderList(); });
  clearSearch.addEventListener("click", () => { query = ""; input.value = ""; renderList(); input.focus(); });
  memory.addEventListener("click", () => {
    const id = filter ?? Agents.activeId;
    void Bridge.memoryOpen(id).catch(err => { error = String(err).replace(/^Error:\s*/, ""); renderList(); });
  });

  function chip(id: string | null, label: string) {
    return h("button", { class: filter === id ? "history-chip on" : "history-chip", text: label,
      onclick: () => { filter = id; confirming = ""; renderChips(); renderList(); } });
  }

  async function open(entry: HistoryEntry) {
    try {
      if (!entry.active) await Bridge.historyOpen(entry.agent, entry.id);
      actions.openChat(entry.agent);
    } catch (err) { error = String(err).replace(/^Error:\s*/, ""); renderList(); }
  }

  async function remove(entry: HistoryEntry) {
    const key = `${entry.agent}/${entry.id}`;
    if (confirming !== key) { confirming = key; renderList(); return; }
    confirming = "";
    try { await Bridge.historyDelete(entry.agent, entry.id); await load(); }
    catch (err) { error = String(err).replace(/^Error:\s*/, ""); renderList(); }
  }

  function row(entry: HistoryEntry, group: HistoryGroup): HTMLElement {
    const agent = Agents.find(entry.agent);
    const name = agent?.name ?? entry.agent;
    const key = `${entry.agent}/${entry.id}`;
    const del = entry.active ? null : h("button", {
      class: confirming === key ? "history-del confirm" : "history-del",
      "aria-label": "Borrar conversación",
      onclick: (e: Event) => { e.stopPropagation(); void remove(entry); },
    }, confirming === key ? h("span", { text: "¿Borrar?" }) : svg(ICONS.trash, 12));
    // The agent's own mascot (its colours, hat and clothes), still.
    const task = State.tasks.find((t) => t.id === agentTaskId(entry.agent));
    const avatar = task ? createMiniBot({ ...task, state: "idle" }, 30, { still: true }) : dot(agent?.color ?? "#E6E9EE", 8);
    return h("div", { class: "history-row", role: "button", tabindex: "0", onclick: () => void open(entry),
      onkeydown: (e: Event) => { if ((e as KeyboardEvent).key === "Enter") void open(entry); } },
      h("span", { class: "history-avatar" }, avatar),
      h("span", { class: "history-text" },
        h("span", { class: "history-title", text: entry.title }),
        h("span", { class: "history-sub", text: `${name} · ${countLabel(entry.count)}` })),
      entry.active ? h("span", { class: "history-open", text: "abierta" }) : null,
      h("span", { class: "history-when", text: rowTime(entry.updated, group) }),
      del ?? h("span", { class: "history-del-slot" }));
  }

  /** The names and colours the list was drawn with. */
  let shownLook = "";

  function renderChips() {
    clear(chips);
    chips.append(chip(null, "Todos"));
    for (const agent of Agents.list) chips.append(chip(agent.id, agent.name));
    memory.setAttribute("aria-label", `Abrir la memoria de ${Agents.find(filter ?? Agents.activeId)?.name ?? ""}`);
  }

  function renderList() {
    shownLook = Agents.lookKey;
    clear(list);
    clearSearch.hidden = !query;
    const inFilter = entries.filter(e => !filter || e.agent === filter);
    const shown = matching(inFilter, query, id => Agents.find(id)?.name ?? id);
    count.textContent = conversationsLabel(shown.length);
    if (error) list.append(empty("failed", error));
    else if (!shown.length) {
      list.append(empty("", !entries.length
        ? (IS_TAURI ? "Todavía no hay conversaciones. Empieza una en el chat." : "Vista previa: el historial vive dentro de MIKA.")
        : "Nada coincide con tu búsqueda."));
    }
    for (const section of groups(shown)) {
      list.append(h("div", { class: "history-section", text: GROUP_TITLES[section.group] }));
      for (const entry of section.entries) list.append(row(entry, section.group));
    }
  }

  function empty(kind: string, text: string) { return h("div", { class: `history-empty ${kind}`.trim(), text }); }

  function render() { renderChips(); renderList(); }

  async function load() {
    error = "";
    if (IS_TAURI) {
      try { entries = await Bridge.historyList(); } catch (err) { error = String(err).replace(/^Error:\s*/, ""); }
    }
    render();
  }

  return {
    el,
    sync: () => {
      if (State.view !== "history") confirming = "";
      else if (shownLook !== Agents.lookKey) render();
    },
    onShow: () => { filter = null; query = ""; input.value = ""; confirming = ""; void load(); },
    preferredHeight: () => 340,
  };
}
