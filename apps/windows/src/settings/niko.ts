// Settings › Finanzas: Niko, the personal-finance agent. Gmail and Notion are the user's own claude.ai connectors
// (Anthropic holds those keys, never Buddy); the mail review runs only on the device where it is switched on. The core
// does all of it; this only shows and asks. Twin of NikoSettings in apps/macos/Sources/Settings/NikoSection.swift.
import { invoke } from "@tauri-apps/api/core";
import { h } from "../chat/dom";
import { agentFace } from "../chat/avatar";
import { SETTINGS_ICONS } from "./icons";
import { button, emptyRow, errorText, header, icon, row, section, toggle } from "./ui";

interface FinanceRecord { key: string; at: number; monto: number; moneda: string; tipo: string; concepto: string; comercio: string; categoria: string; origen: string; recurrente: boolean }
interface NikoStatus {
  enabled: boolean; interval: number; senders: string; parentPage: string; dashboardUrl: string; telegram: boolean; agentReady: boolean;
  running: boolean; lastRunAt: number; lastOk: boolean; lastError: string; lastRecorded: number; recent: FinanceRecord[];
}
interface AccountStatus { id: string; name: string; state: string }

const CONNECTORS = "https://claude.ai/settings/connectors";
const INTERVALS = [10, 20, 30, 60];

/** «S/ 45,90», «US$ 12,99» (the core formats the notices the same way). */
export function money(amount: number, currency: string): string {
  const [whole, cents] = Math.abs(amount).toFixed(2).split(".");
  const grouped = whole!.replace(/\B(?=(\d{3})+(?!\d))/g, " ");
  return `${amount < 0 ? "-" : ""}${currency === "USD" ? "US$" : "S/"} ${grouped},${cents}`;
}

const STATE_TEXT: Record<string, string> = {
  connected: "Conectado",
  "needs-auth": "Falta autorizar",
  missing: "No disponible en esta conexión",
};

function when(at: number): string {
  return new Intl.DateTimeFormat("es-PE", { timeZone: "America/Lima", day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" }).format(new Date(at * 1000));
}

function ago(at: number): string {
  const minutes = Math.max(0, Math.round((Date.now() / 1000 - at) / 60));
  if (minutes < 1) return "Hace un momento";
  if (minutes < 60) return `Hace ${minutes} min`;
  const hours = Math.round(minutes / 60);
  return hours < 24 ? `Hace ${hours} h` : `Hace ${Math.round(hours / 24)} d`;
}

export async function renderNiko(view: HTMLElement): Promise<(e: { type: string }) => void> {
  const error = h("p", { class: "error small", role: "alert" });
  const note = h("p", { class: "muted small", role: "status" });
  const fail = (e: unknown) => { error.textContent = errorText(e); };

  // Intro with Niko's face.
  const face = h("span", { class: "avatar", "aria-hidden": "true" }, icon(SETTINGS_ICONS.userCircle, 20));
  void agentFace("niko").then((url) => { if (url) face.replaceChildren(h("img", { src: url, alt: "", class: "pixel" })); });
  const intro = section(null, h("div", { class: "row agent" }, face,
    h("div", { class: "row-text" },
      h("div", { class: "row-title", text: "«Entiende tu plata, no solo la anotes»" }),
      h("div", { class: "row-detail", text: "Niko anota en tu Notion cada movimiento: los correos de tus bancos y apps, lo que le escribas («gasté 45 en almuerzo») y las fotos de tus vouchers de Yape o Plin. Nunca mueve dinero ni responde correos." }))));
  const permission = h("p", { class: "error small", text: "" });
  intro.card.append(permission);

  // claude.ai accounts.
  const accounts = section("Claude y GPT · cambio automático");
  const accountList = h("div", { class: "list" }, emptyRow("Comprobando Claude y GPT…"));
  const recheck = button("Volver a comprobar", () => void checkAccounts(), "secondary");
  accounts.card.append(accountList, h("div", { class: "toolbar end" }, recheck));
  accounts.el.append(h("p", { class: "muted small", text: "Usa tus conexiones de Claude y ChatGPT. Si uno se queda sin cuota, Niko continúa con el otro y conserva las mismas bases de Notion. Buddy no guarda esas claves. El acceso al correo se confirma al revisarlo." }));

  async function checkAccounts() {
    recheck.disabled = true;
    accountList.replaceChildren(emptyRow("Comprobando Claude y GPT…"));
    const list = await invoke<AccountStatus[]>("niko_accounts").catch(() => [] as AccountStatus[]);
    recheck.disabled = false;
    if (!list.length) { accountList.replaceChildren(emptyRow("No se pudieron comprobar las conexiones.", "Inicia sesión en Claude Code o Codex.")); return; }
    accountList.replaceChildren(...list.map((a) => {
      const ok = a.state === "connected";
      const detail = h("div", { class: "row-detail", text: STATE_TEXT[a.state] ?? "No responde", style: ok ? undefined : "color: var(--color-warning)" });
      const url = a.id.startsWith("codex:") ? "https://chatgpt.com" : CONNECTORS;
      return row(a.name, detail, ok ? null : button(a.id.startsWith("codex:") ? "Revisar en ChatGPT" : "Autorizar en Claude", () => void invoke("open_url", { url }), "ghost", url));
    }));
  }

  // The mail review.
  const enabled = toggle("Niko revisa el correo en este equipo", false, (on) => void invoke("niko_set_enabled", { on }).catch(fail).then(draw));
  const interval = h("select", { class: "select", "aria-label": "Cada cuántos minutos" },
    ...INTERVALS.map((m) => h("option", { value: String(m), text: `${m} minutos` })));
  interval.addEventListener("change", () => void invoke("niko_set_interval", { minutes: Number(interval.value) }).catch(fail).then(draw));
  const senders = h("textarea", { rows: 3, "aria-label": "Remitentes", spellcheck: "false" });
  const telegram = toggle("Avisarme también por Telegram", false, (on) => void invoke("niko_set_telegram", { on }).catch(fail).then(draw));
  const review = section("Revisión del correo",
    row("Niko revisa el correo en este equipo", "Déjalo encendido en un solo equipo (este PC o tu Mac). Usa Haiku o GPT Luna con esfuerzo bajo para revisar rápido y gastar poco.", enabled),
    row("Cada", null, interval),
    h("div", { class: "field" },
      h("div", { class: "row-title", text: "Remitentes (dominios o correos)" }),
      senders,
      h("div", { class: "footer-line" },
        h("span", { class: "muted small", text: "Separados por comas. Vacío vuelve a los bancos y apps de siempre." }),
        button("Guardar", () => void invoke<string>("niko_set_senders", { senders: senders.value })
          .then((kept) => { senders.value = kept; note.textContent = "Remitentes guardados."; }, fail), "primary"))),
    row("Avisarme también por Telegram", "Lo que anota y los avisos de presupuesto, en tu chat vinculado.", telegram));

  // Notion.
  const parent = h("input", { class: "text", type: "url", placeholder: "https://www.notion.so/Buddy-Finanzas-…", "aria-label": "Enlace de la página de Notion", spellcheck: "false", style: "flex: 1; width: auto" });
  const open = button("Abrir dashboard", () => { if (status?.dashboardUrl) void invoke("open_url", { url: status.dashboardUrl }); }, "secondary");
  const refresh = button("Actualizar dashboard", () => { void invoke("niko_refresh_dashboard"); note.textContent = "Actualizando el dashboard en Notion…"; }, "ghost", "Lee el mes en Notion y vuelve a escribir los totales");
  const notion = section("Notion",
    h("div", { class: "field" },
      h("div", { class: "row-title", text: "Página donde Niko guarda todo" }),
      h("div", { class: "actions" }, parent, button("Guardar", () => {
        error.textContent = "";
        void invoke("niko_set_parent", { page: parent.value }).then(() => { note.textContent = "Página guardada."; }, fail).then(draw);
      }, "primary")),
      h("p", { class: "muted small", text: "Vacío: Niko busca una página llamada «Buddy · Finanzas». Compártela con el conector de Notion." })),
    h("div", { class: "toolbar" }, open, refresh));

  // The last review and what it recorded.
  const last = h("div", { class: "row-detail" });
  const now = button("Revisar ahora", () => void invoke("niko_review_now"), "primary");
  const recent = h("div", { class: "list" });
  const lastSection = section("Última revisión", row("Estado", last, now), recent);

  let status: NikoStatus | null = null;
  async function draw() {
    const s = await invoke<NikoStatus>("niko_status").catch(() => null);
    if (!s) return;
    status = s;
    permission.textContent = s.agentReady ? "" : "Niko necesita el permiso «Cuentas» (Ajustes › Agentes).";
    enabled.setAttribute("aria-checked", String(s.enabled));
    telegram.setAttribute("aria-checked", String(s.telegram));
    interval.value = String(s.interval);
    open.disabled = refresh.disabled = !s.dashboardUrl;
    now.disabled = s.running;
    last.style.color = !s.lastOk && s.lastRunAt > 0 && !s.running ? "var(--color-warning)" : "";
    last.textContent = s.running ? "Revisando…"
      : s.lastRunAt === 0 ? "Todavía no revisó el correo."
      : !s.lastOk ? `${ago(s.lastRunAt)}: ${s.lastError}`
      : s.lastRecorded === 0 ? `${ago(s.lastRunAt)}: nada nuevo.` : `${ago(s.lastRunAt)}: anotó ${s.lastRecorded}.`;
    recent.replaceChildren(...s.recent.map((r) =>
      row(`${money(r.monto, r.moneda)} · ${r.comercio || r.concepto}`, `${r.tipo} · ${r.categoria}`, h("span", { class: "muted small", text: when(r.at) }))));
  }

  view.append(header("Finanzas", "Niko · tus gastos, pagos y suscripciones en Notion"), intro.el, accounts.el, review.el, notion.el, lastSection.el, note, error);
  await draw();
  const first = status as NikoStatus | null;
  senders.value = first?.senders ?? "";
  parent.value = first?.parentPage ?? "";
  void checkAccounts();
  return (e) => { if (e.type === "nikoChanged") void draw(); };
}
