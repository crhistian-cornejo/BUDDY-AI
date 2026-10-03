// Settings › Agentes: an agent's face («cara») — hood colour, accessory, eyes and the accessory's colour — with a
// live preview at 4×. The core draws it (agent_sprite) and keeps it (agent.<id>.cara); this only chooses.
// Twin of AgentFaceEditor in apps/macos/Sources/Settings/AgentFaceEditor.swift.
import { invoke } from "@tauri-apps/api/core";
import { h } from "../chat/dom";
import { agentFace, forgetFace } from "../chat/avatar";
import { button } from "./ui";

export interface AgentLook { color: string; accessory: string; eyes: string; badge: string | null }
interface LookOption { id: string; label: string; hex: string | null }
export interface LookOptions { colors: LookOption[]; accessories: LookOption[]; eyes: LookOption[] }

let options: Promise<LookOptions> | null = null;

/** The choices (Spanish labels from the core), asked once. */
export function lookOptions(): Promise<LookOptions> {
  options ??= invoke<LookOptions>("look_options").catch(() => {
    options = null;
    return { colors: [], accessories: [], eyes: [] };
  });
  return options;
}

/** Round colour swatches as a radio group; the chosen one has a ring and a check. */
function swatches(label: string, list: LookOption[], selected: string | null, pick: (id: string) => void, small = false): HTMLElement {
  return h("div", { class: `swatches${small ? " small" : ""}`, role: "radiogroup", "aria-label": label },
    ...list.map((c) => {
      const on = c.id === selected;
      const el = h("button", { type: "button", class: "swatch", role: "radio", "aria-checked": String(on), title: c.label, "aria-label": c.label, style: `--swatch:${c.hex ?? "#888"}` });
      el.addEventListener("click", () => pick(c.id));
      return el;
    }));
}

/** The face editor for one agent; `onChange` runs after a face is saved (to refresh the header avatar). */
export function faceEditor(agentId: string, agentName: string, onChange: () => void): HTMLElement {
  const box = h("div", { class: "face-editor" });
  const preview = h("img", { class: "face-preview pixel", alt: `Cara de ${agentName}` });
  const status = h("span", { class: "muted small", role: "status" });

  async function save(look: AgentLook | null) {
    try {
      if (look) await invoke("set_agent_look", { agentId, look });
      else await invoke("reset_agent_look", { agentId });
      status.textContent = "";
    } catch (e) {
      status.textContent = `No se pudo guardar: ${String(e)}`;
    }
    forgetFace(agentId);
    await draw();
    onChange();
  }

  async function draw() {
    const [opts, look] = await Promise.all([lookOptions(), invoke<AgentLook>("agent_look", { agentId }).catch(() => null)]);
    if (!look || !opts.colors.length) { box.replaceChildren(h("span", { class: "muted small", text: "Sin cara" })); return; }
    void agentFace(agentId).then((url) => { if (url) preview.src = url; });
    const set = (patch: Partial<AgentLook>) => void save({ ...look, ...patch });

    const accessory = h("select", { class: "select", "aria-label": `Accesorio de ${agentName}`, title: "Lo que lleva en la cabeza o la cara" },
      ...opts.accessories.map((a) => h("option", { value: a.id, text: a.label, selected: a.id === look.accessory })));
    accessory.addEventListener("change", () => set({ accessory: accessory.value }));

    const eyes = h("div", { class: "segmented", role: "radiogroup", "aria-label": `Ojos de ${agentName}`, title: "Cómo mira" },
      ...opts.eyes.map((e) => {
        const el = h("button", { type: "button", role: "radio", "aria-checked": String(e.id === look.eyes), text: e.label });
        el.addEventListener("click", () => set({ eyes: e.id }));
        return el;
      }));

    const badge = look.accessory === "ninguno" ? null : h("div", { class: "face-line" },
      h("span", { class: "muted small", text: "Color del accesorio" }),
      (() => {
        const auto = h("button", { type: "button", class: "chip-auto", role: "radio", "aria-checked": String(!look.badge), text: "Auto", title: "Uno que resalte sobre la capucha" });
        auto.addEventListener("click", () => set({ badge: null }));
        return auto;
      })(),
      swatches(`Color del accesorio de ${agentName}`, opts.colors, look.badge, (id) => set({ badge: id }), true));

    const reset = button("Restablecer", () => void save(null), "ghost", "Vuelve a la cara de su archivo agent.md");
    reset.classList.add("small");

    box.replaceChildren(
      h("div", { class: "face-frame" }, preview),
      h("div", { class: "face-controls" },
        swatches(`Color de ${agentName}`, opts.colors, look.color, (id) => set({ color: id })),
        h("div", { class: "face-line" }, accessory, eyes),
        badge,
        h("div", { class: "face-line" }, reset, status)));
  }

  void draw();
  return box;
}
