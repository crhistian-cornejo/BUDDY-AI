// Settings › Conexiones › Conectores (MCP): Buddy's built-in remote MCP servers (nothing to install), the same as
// the Mac's ConnectorsSection.swift. A switch each; the optional key goes only to Credential Manager. The core does all of it.
import { invoke } from "@tauri-apps/api/core";
import { h } from "../chat/dom";
import { button, emptyRow, errorText, row, section, toggle } from "./ui";

interface ConnectorInfo { id: string; name: string; description: string; site: string; enabled: boolean; hasKey: boolean; keyOptional: boolean }

export function renderConnectors(): HTMLElement {
  const { el, card } = section("Conectores (MCP)");
  el.append(h("p", { class: "muted small", text: "Vienen con Buddy y no instalan nada. Los usan Claude, Codex y Gemini en los agentes que pueden buscar en la web. La clave solo da más uso y se guarda en el Administrador de credenciales; con Gemini van sin clave." }));
  const error = h("p", { class: "error small", role: "alert" });

  async function draw() {
    const items = await invoke<ConnectorInfo[]>("connectors").catch(() => null);
    if (!items) { card.replaceChildren(emptyRow("No se pudieron leer los conectores.")); return; }
    card.replaceChildren(...items.map(item), error);
  }

  function item(c: ConnectorInfo): HTMLElement {
    const sw = toggle(c.name, c.enabled, (on) => {
      error.textContent = "";
      void invoke("set_connector_enabled", { id: c.id, on }).catch((e) => { error.textContent = errorText(e); }).then(draw);
    });
    const site = button("Sitio", () => void invoke("open_url", { url: c.site }), "ghost", c.site);
    const top = row(c.name, c.description, site, sw);
    if (!c.keyOptional) return top;
    const key = h("input", { class: "text", type: "password", placeholder: "Clave (opcional)", "aria-label": `Clave de ${c.name} (opcional)`, autocomplete: "off", spellcheck: "false", style: "flex: 1; width: auto" });
    const save = (value: string) => {
      error.textContent = "";
      void invoke("set_connector_key", { id: c.id, key: value }).catch((e) => { error.textContent = errorText(e); }).then(draw);
    };
    const saveButton = button("Guardar", () => { if (key.value.trim()) save(key.value); }, "primary");
    const remove = c.hasKey ? button("Quitar", () => save("")) : null;
    return h("div", {},
      top,
      h("div", { class: "field" },
        c.hasKey ? h("p", { class: "muted small", text: "Clave guardada en el Administrador de credenciales." }) : null,
        h("div", { class: "actions" }, key, saveButton, remove)));
  }

  void draw();
  return el;
}
