// Settings › Agentes: who Buddy can hand work to, and a way to their agent.md files. Kept on its own so the per-agent
// appearance can grow here later. Twin of AgentSettings in apps/macos/Sources/Settings/SettingsView.swift.
import { invoke } from "@tauri-apps/api/core";
import { h } from "../chat/dom";
import { buddyFace } from "../chat/avatar";
import { SETTINGS_ICONS } from "./icons";
import { button, emptyRow, errorText, header, icon, providerMark, section } from "./ui";

export interface Agent { id: string; name: string; specialty: string; provider: string; model: string | null }

function avatar(agent: Agent): HTMLElement {
  const box = h("span", { class: "avatar", "aria-hidden": "true" }, icon(SETTINGS_ICONS.userCircle, 20));
  if (agent.id === "buddy") {
    void buddyFace().then((url) => { if (url) box.replaceChildren(h("img", { src: url, alt: "", class: "pixel" })); });
  }
  return box;
}

function agentRow(agent: Agent): HTMLElement {
  return h("div", { class: "row agent" },
    avatar(agent),
    h("div", { class: "row-text" },
      h("div", { class: "row-title with-mark" }, h("span", { text: agent.name }), providerMark(agent.provider, 11)),
      h("div", { class: "row-detail", text: agent.specialty })));
}

export async function renderAgents(view: HTMLElement): Promise<void> {
  const agents = await invoke<Agent[]>("agents").catch(() => [] as Agent[]);
  const note = h("p", { class: "muted small", role: "status" });
  const open = button("Abrir carpeta de agentes", () => {
    void invoke("open_agents_folder").then(() => { note.textContent = ""; }, (e) => { note.textContent = `No se pudo abrir: ${errorText(e)}`; });
  });
  view.append(
    header("Agentes"),
    section(null, ...(agents.length ? agents.map(agentRow) : [emptyRow("Sin agentes")])).el,
    h("div", { class: "footer-line" }, h("span", { class: "muted small", text: "Cada agente es un archivo agent.md que puedes editar." }), open),
    note,
  );
}
