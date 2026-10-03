import { invoke } from "@tauri-apps/api/core";
import { h } from "../chat/dom";
import { button, errorText, section } from "./ui";

interface Chat { id: number; title: string; kind: string }
interface Post { chatId: number; chat: string; id: number; date: number; text: string; photos: string[] }
interface Account { apiId?: number; configured: boolean; authorized: boolean; step: string; name?: string; hint?: string; chats: Chat[]; selected: number[]; posts: Post[]; errors?: string[] }

/** Separate from the bot: the user's account, read only and only on an explicit click. */
export function renderTelegramAccount(): HTMLElement {
  const { el, card } = section("Telegram · Cuenta personal");
  el.append(h("p", { class: "muted small", text: "Consultar mensajes muestra los últimos 20 por grupo. Analizar hoy con PARLEY vuelve a leer los mensajes de hoy con hora de Lima, hasta 200 por grupo, y verifica qué picks quedan pendientes. Solo lee los grupos elegidos; no envía mensajes ni los marca como leídos. Las credenciales y la sesión van al Administrador de credenciales. El análisis envía los mensajes y hasta 10 fotos al proveedor del agente; informa cualquier lectura parcial." }));
  let state: Account = { configured: false, authorized: false, step: "idle", chats: [], selected: [], posts: [] };
  let editingCredentials = false;
  let busy = false;
  let problem = "";
  let analysis = "";
  let phone = "";
  const field = (placeholder: string, type = "text") => h("input", { type, placeholder, autocomplete: "off" });

  async function request(action: string, value = "") {
    if (busy) return;
    busy = true;
    problem = "";
    for (const control of card.querySelectorAll<HTMLButtonElement | HTMLInputElement>("button,input")) control.disabled = true;
    const progress = h("p", { class: "muted small", text: "Conectando con Telegram…" });
    card.append(progress);
    try {
      const reply = JSON.parse(await invoke<string>("telegram_account_request", { action, value }));
      if (action === "analyze") analysis = reply.analysis ?? "";
      else { state = reply; if (action === "configure") editingCredentials = false; if (["select", "fetch", "signOut"].includes(action)) analysis = ""; }
    } catch (error) { problem = errorText(error); }
    finally { busy = false; draw(); }
  }
  function draw() {
    const content: Node[] = [];
    if (!state.configured || editingCredentials) {
      const id = field("api_id", "number");
      id.value = state.apiId?.toString() ?? "";
      const hash = field(state.configured ? "api_hash (vacío conserva el guardado)" : "api_hash", "password");
      content.push(h("p", { class: "muted small", text: "Conecta tu cuenta para consultar los picks de los grupos y canales que elijas." }),
        button("Obtener api_id y api_hash", () => void invoke("open_url", { url: "https://my.telegram.org/apps" })), id, hash,
        button("Guardar y continuar", () => void request("configure", JSON.stringify({ id: Number(id.value), hash: hash.value })), "primary"));
      if (editingCredentials) content.push(button("Cancelar", () => { editingCredentials = false; draw(); }));
    } else if (!state.authorized) {
      content.push(button("Editar api_id / api_hash", () => { editingCredentials = true; draw(); }));
      if (state.step === "password") {
        const password = field("Contraseña de verificación en dos pasos", "password");
        content.push(h("p", { text: state.hint ?? "Telegram pide tu contraseña de dos pasos." }), password,
          button("Iniciar sesión", () => void request("password", password.value), "primary"));
      } else if (state.step === "code") {
        const code = field("Código recibido en Telegram", "password");
        content.push(code, button("Verificar código", () => void request("signIn", code.value), "primary"));
        if (phone) content.push(button("Volver a pedir código", () => void request("sendCode", phone)));
      } else {
        const input = field("Teléfono con código de país (+51…)");
        input.value = phone;
        content.push(input, button("Pedir código", () => { phone = input.value; void request("sendCode", phone); }, "primary"));
      }
    } else {
      content.push(h("p", { text: `Cuenta: ${state.name ?? "Conectada"}` }),
        h("div", { class: "actions" }, button("Elegir grupos", () => void request("chats")), button("Cerrar sesión", () => void request("signOut"))));
      for (const chat of state.chats) {
        const checked = state.selected.includes(chat.id);
        const checkbox = h("input", { type: "checkbox", checked, disabled: !checked && state.selected.length >= 10,
          onchange: () => void request("select", JSON.stringify(checked ? state.selected.filter(id => id !== chat.id) : [...state.selected, chat.id])) });
        content.push(h("label", {}, checkbox, chat.title));
      }
      if (state.selected.length) content.push(h("p", { class: "muted small", text: `${state.selected.length} grupos seleccionados (máximo 10)` }),
        h("div", { class: "actions" }, button("Consultar mensajes", () => void request("fetch")),
          button("Analizar hoy con PARLEY", () => void request("analyze"), "primary")));
      if (analysis) content.push(h("details", { open: true }, h("summary", { text: "Análisis de PARLEY" }), h("p", { style: "white-space:pre-wrap", text: analysis }), h("p", { class: "muted small", text: "También está en el historial: Telegram · Grupos" })));
      if (state.posts.length) {
        const messages = h("details", {}, h("summary", { text: `Mensajes consultados (${state.posts.length})` }));
        for (const post of state.posts.slice(-30).reverse()) messages.append(h("div", { class: "field" },
          h("strong", { text: post.chat }), h("span", { class: "muted small", text: new Date(post.date * 1000).toLocaleString() }),
          h("p", { style: "white-space:pre-wrap", text: post.text || "Mensaje sin texto" }),
          post.photos.length ? h("span", { class: "muted small", text: `${post.photos.length} foto(s) disponibles para PARLEY` }) : null));
        content.push(messages);
      }
    }
    if (problem) content.push(h("p", { class: "error small", text: problem }));
    for (const error of state.errors ?? []) content.push(h("p", { class: "error small", text: error }));
    card.replaceChildren(h("div", { class: "field" }, ...content));
  }
  draw();
  void request("status");
  return el;
}
