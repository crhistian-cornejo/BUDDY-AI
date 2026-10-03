// Settings › Agentes: each agent's model (the router, or one fixed), what it may do and its face, plus a way to their
// agent.md files. Twin of AgentSettings in apps/macos/Sources/Settings/SettingsView.swift.
import { invoke } from "@tauri-apps/api/core";
import { h } from "../chat/dom";
import { agentFace } from "../chat/avatar";
import { renderNiko } from "./niko";
import { faceEditor } from "./face-editor";
import { SETTINGS_ICONS } from "./icons";
import { button, emptyRow, errorText, header, icon, providerMark, row, section } from "./ui";

export interface Agent { id: string; name: string; specialty: string; provider: string; model: string | null; permissions: string[] }
interface PermissionInfo { id: string; name: string; detail: string }
interface ModelOption { id: string; name: string; provider: string }

function avatar(agent: Agent): HTMLElement {
  const box = h("span", { class: "avatar", "aria-hidden": "true" }, icon(SETTINGS_ICONS.userCircle, 20));
  void agentFace(agent.id).then((url) => { if (url) box.replaceChildren(h("img", { src: url, alt: "", class: "pixel" })); });
  return box;
}

function title(agent: Agent): HTMLElement {
  return h("div", { class: "row agent" },
    avatar(agent),
    h("div", { class: "row-text" },
      h("div", { class: "row-title with-mark" }, h("span", { text: agent.name }), providerMark(agent.provider, 11)),
      h("div", { class: "row-detail", text: agent.specialty })));
}

export async function renderAgents(view: HTMLElement): Promise<(e: { type: string }) => void> {
  let selected = "buddy";
  let revision = 0;
  let onNikoEvent: ((e: { type: string }) => void) | undefined;
  const tabs = h("div", { class: "segmented agent-tabs", role: "tablist", "aria-label": "Agentes" });
  const note = h("p", { class: "muted small", role: "status" });
  const list = h("div", { class: "agent-list" });
  const open = button("Abrir carpeta de agentes", () => {
    void invoke("open_agents_folder").then(() => { note.textContent = ""; }, (e) => { note.textContent = `No se pudo abrir: ${errorText(e)}`; });
  });
  view.append(
    header("Agentes"),
    tabs,
    list,
    h("div", { class: "footer-line" }, h("span", { class: "muted small", text: "Cada agente es un archivo agent.md que puedes editar. Ejecutar comandos siempre pide tu clic." }), open),
    note,
  );

  async function draw() {
    const [agents, catalog, config] = await Promise.all([
      invoke<Agent[]>("agents").catch(() => [] as Agent[]),
      invoke<PermissionInfo[]>("agent_permission_catalog").catch(() => [] as PermissionInfo[]),
      invoke<{ models: ModelOption[] }>("router_config").catch(() => ({ models: [] as ModelOption[] })),
    ]);
    if (!agents.length) { list.replaceChildren(section(null, emptyRow("Sin agentes")).el); return; }
    if (!agents.some((a) => a.id === selected)) selected = agents[0]!.id;
    const version = ++revision;
    onNikoEvent = undefined;
    tabs.replaceChildren(...agents.map((a) => {
      const tab = h("button", { type: "button", role: "tab", "aria-selected": String(a.id === selected), text: a.name });
      tab.addEventListener("click", () => { selected = a.id; void draw(); });
      return tab;
    }));
    tabs.onkeydown = (event) => {
      const step = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
      if (!step) return;
      event.preventDefault();
      selected = agents[(agents.findIndex((a) => a.id === selected) + step + agents.length) % agents.length]!.id;
      void draw().then(() => tabs.querySelector<HTMLButtonElement>('[aria-selected="true"]')?.focus());
    };
    list.replaceChildren(...agents.filter((a) => a.id === selected).map((agent) => {
      const fail = (e: unknown) => { note.textContent = `No se pudo: ${errorText(e)}`; };
      let modelControl: Node;
      if (agent.id === "buddy") {
        modelControl = h("span", { class: "muted small", text: "El de «Modelo de Buddy» en General" });
      } else {
        const known = agent.model === "auto" || config.models.some((m) => m.id === agent.model);
        const select = h("select", { class: "select", "aria-label": `Modelo de ${agent.name}` },
          h("option", { value: "auto", text: "Automático (el router decide)", selected: agent.model === "auto" }),
          ...config.models.map((m) => h("option", { value: m.id, text: m.name, selected: m.id === agent.model })),
          !known && agent.model ? h("option", { value: "", text: `El de su archivo (${agent.model})`, selected: true }) : null);
        select.addEventListener("change", () => void invoke("set_agent_model", { agentId: agent.id, model: select.value }).then(draw, fail));
        modelControl = select;
      }
      const checks = h("div", { class: "perm-grid" }, ...catalog.map((p) => {
        const box = h("input", { type: "checkbox", checked: agent.permissions.includes(p.id), "aria-describedby": `perm-${agent.id}-${p.id}` });
        box.addEventListener("change", () => {
          const next = agent.permissions.filter((x) => x !== p.id).concat(box.checked ? [p.id] : []);
          void invoke("set_agent_permissions", { agentId: agent.id, permissions: next }).then(draw, fail);
        });
        return h("label", { class: "perm", title: p.detail }, box, h("span", { id: `perm-${agent.id}-${p.id}`, text: p.name }));
      }));
      const head = title(agent);
      const face = row("Cara", null, faceEditor(agent.id, agent.name, () => head.querySelector(".avatar")?.replaceWith(avatar(agent))));
      face.classList.add("face-row");
      return section(null, head, row("Modelo", null, modelControl), row("Puede", null, checks), face).el;
    }));
    if (selected === "niko") {
      const finance = h("div", { class: "agent-finance" });
      list.append(finance);
      const handler = await renderNiko(finance);
      if (version === revision) onNikoEvent = handler;
    }
  }

  await draw();
  return (event) => onNikoEvent?.(event);
}
