import { invoke } from "@tauri-apps/api/core";
import { h } from "../chat/dom";
import { button, errorText, row, section } from "./ui";

interface OddsStatus { enabled: boolean; hasKey: boolean; bookmaker: string; dailyCap: number }

export function renderParleyOdds(): HTMLElement {
  const { el, card } = section("PARLEY · Cuotas");
  el.append(h("p", { class: "muted small", text: "PARLEY consulta el calendario y cuotas de Betano Perú en OddsPapi. Antes de analizar también revisa el bot y los mensajes de hoy de tus grupos seleccionados en Telegram." }));
  const note = h("p", { class: "small", role: "status" });
  let busy = false;
  async function request(action: string, value = "") {
    if (busy) return;
    busy = true;
    try {
      const status = JSON.parse(await invoke<string>("parley_odds_request", { action, value })) as OddsStatus;
      draw(status);
      if (action === "configure") note.textContent = value ? "Clave guardada en el Administrador de credenciales. Se verificará en la próxima consulta." : "Clave retirada.";
    } catch (e) { note.textContent = errorText(e); }
    finally { busy = false; }
  }
  function draw(status: OddsStatus) {
    const key = h("input", { class: "text", type: "password", placeholder: "Clave de OddsPapi", "aria-label": "Clave de OddsPapi", autocomplete: "off", spellcheck: "false" });
    const save = button("Guardar", () => { if (key.value.trim()) void request("configure", key.value); }, "primary");
    card.replaceChildren(
      row(status.hasKey && status.enabled ? "OddsPapi configurado · Betano Perú" : "Falta configurar OddsPapi", `Máximo ${status.dailyCap} consultas al día. Caché de 5 minutos.`),
      h("div", { class: "field" }, key, h("div", { class: "actions" }, save,
        status.hasKey ? button("Quitar clave", () => void request("configure")) : null,
        button("Obtener clave", () => void invoke("open_url", { url: "https://oddspapi.io/en/account" })))),
      note);
  }
  card.append(note);
  void request("status");
  return el;
}
