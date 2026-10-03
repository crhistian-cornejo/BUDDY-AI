// Ajustes › Conexiones › iPhone: the relay's address and key, pairing a phone by QR, forgetting it, the switch for
// approving from the phone, and what the phone did on this PC. The core does all of it; this only draws.
import { invoke } from "@tauri-apps/api/core";
import { h } from "../chat/dom";
import { button, section } from "./ui";

interface RemoteStatus { relay: string; hasOwnerKey: boolean; paired: boolean; phone: string; connected: boolean; online: boolean; approvals: boolean; pairing: boolean; error: string }
interface PairOffer { uri: string; size: number; cells: boolean[]; expiresAt: number }
interface RemoteAction { at: number; text: string }

/** Paints the core's QR matrix (`size` by `size`, row by row; true is dark): always dark on white, with its quiet margin. */
export function paintQr(canvas: HTMLCanvasElement, size: number, cells: boolean[]) {
  const quiet = 4;
  const module = Math.max(2, Math.floor(220 / (size + quiet * 2)));
  canvas.width = canvas.height = module * (size + quiet * 2);
  const context = canvas.getContext("2d");
  if (!context) return;
  context.fillStyle = "#fff";
  context.fillRect(0, 0, canvas.width, canvas.height);
  if (size <= 0 || cells.length !== size * size) return;
  context.fillStyle = "#000";
  for (let row = 0; row < size; row++) {
    for (let column = 0; column < size; column++) {
      if (cells[row * size + column]) context.fillRect((column + quiet) * module, (row + quiet) * module, module, module);
    }
  }
}

export function renderPhone(): { element: HTMLElement; refresh: () => Promise<void> } {
  const { el, card } = section("iPhone");
  const state = h("p", { role: "status", class: "row-title" });
  const detail = h("p", { class: "muted small" });
  const message = h("p", { role: "status", class: "muted small" });
  const relay = h("input", { class: "text", type: "text", placeholder: "https://buddy-relay.ejemplo.workers.dev", "aria-label": "Dirección del relé", autocomplete: "off", spellcheck: "false" }) as HTMLInputElement;
  const ownerKey = h("input", { class: "text", type: "password", placeholder: "Clave del relé", "aria-label": "Clave del relé", autocomplete: "off" }) as HTMLInputElement;
  const qr = h("canvas", { "aria-label": "Código para emparejar el iPhone", role: "img", hidden: true, style: "display:block;margin:8px auto;border-radius:8px" }) as HTMLCanvasElement;
  const qrNote = h("p", { class: "muted small", hidden: true, text: "Vale 5 minutos y sirve una sola vez." });
  const approvals = h("input", { type: "checkbox", "aria-label": "Aprobar permisos desde el iPhone" }) as HTMLInputElement;
  const approvalsRow = h("label", { class: "small", style: "display:flex;gap:8px;align-items:center" }, approvals,
    "Aprobar permisos desde el iPhone (apagado, el teléfono solo puede rechazar)");
  const log = h("div", { class: "muted small" });
  let offered = false;

  async function run(work: () => Promise<unknown>) {
    try { await work(); message.textContent = ""; } catch (error) { message.textContent = String(error); }
    await refresh();
  }
  const pair = button("Emparejar", () => void run(async () => {
    await invoke("remote_set_relay", { url: relay.value, ownerKey: ownerKey.value });
    const offer = await invoke<PairOffer>("remote_pair");
    ownerKey.value = "";
    paintQr(qr, offer.size, offer.cells);
    offered = true;
  }));
  const forget = button("Olvidar iPhone", () => void run(() => invoke("remote_forget")));
  approvals.addEventListener("change", () => void run(() => invoke("remote_set_approvals", { on: approvals.checked })));
  const setup = h("div", {},
    h("p", { class: "small", style: "white-space:pre-line", text: "1. Despliega tu relé (carpeta relay/ del proyecto) y pega aquí su dirección y su clave de dueño.\n2. Pulsa Emparejar y escanea el código con la app de Buddy en el iPhone." }),
    relay, ownerKey, qr, qrNote, h("div", { class: "row-btns" }, pair));
  const paired = h("div", { hidden: true }, approvalsRow, h("div", { class: "row-btns" }, forget), log);
  card.append(state, detail, setup, paired, message);
  el.append(h("p", { class: "muted small", text: "Todo viaja cifrado de extremo a extremo: el relé solo reenvía bytes que no puede leer. Las claves se guardan solo en el Administrador de credenciales. Desde el teléfono no se cambian ajustes, claves ni carpetas." }));

  async function refresh() {
    const status = await invoke<RemoteStatus>("remote_status");
    state.textContent = status.paired ? (status.phone || "iPhone emparejado") : "Sin emparejar";
    detail.textContent = status.error || (!status.paired ? "" : status.online ? "Conectado ahora" : status.connected ? "Emparejado; recibe avisos mientras no está conectado" : "Sin conexión con el relé");
    setup.hidden = status.paired;
    paired.hidden = !status.paired;
    if (!relay.value) relay.value = status.relay;
    ownerKey.placeholder = status.hasOwnerKey ? "Clave del relé (guardada)" : "Clave del relé";
    const showing = offered && status.pairing;
    qr.hidden = qrNote.hidden = !showing;
    pair.textContent = showing ? "Nuevo código" : "Emparejar";
    approvals.checked = status.approvals;
    if (status.paired) {
      const actions = await invoke<RemoteAction[]>("remote_log");
      log.replaceChildren(...actions.slice(0, 30).map((a) =>
        h("p", { text: `${new Date(a.at * 1000).toLocaleString("es", { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" })} · ${a.text}` })));
    }
  }
  void refresh();
  return { element: el, refresh };
}
