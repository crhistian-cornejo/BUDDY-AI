import { renderYouTube } from "./youtube";
// Settings share the Mac layout: General, Carpetas, Conexiones, Uso and Agentes.
// Novedades live in General; each agent has a tab, with Finanzas inside Niko.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { applyTokens } from "../tokens";
import { h } from "../chat/dom";
import { TABLER } from "../chat/tabler";
import { renderAgents } from "./agents";
import { usageCalendar, type TokenDay } from "./usage-calendar";
import { renderTelegramAccount } from "./telegram-account";
import { renderPhone } from "./phone";
import { renderConnectors } from "./connectors";
import { renderParleyOdds } from "./parley";
import { SETTINGS_ICONS } from "./icons";
import { button, emptyRow, errorText, header, icon, iconButton, k, providerMark, row, section, settingRow } from "./ui";

applyTokens();

type CoreEvent = { type: string };
interface AuthorizedFolder { path: string; canEdit: boolean }
interface HookStatusInfo { agent: string; name: string; installed: boolean; available: boolean; relayReady: boolean }
interface HookPreview { path: string; diff: string; fingerprint: string }
interface UsageWindow { label: string; usedPct: number; resetsAt: number | null }
interface ProviderUsage { provider: string; name: string; windows: UsageWindow[] }
interface TokenReport { feature: string; provider: string; turns: number; input: number; output: number; cached: number; costUsd: number }
interface BriefingItem { topic: string; text: string; url: string | null; at: number }
interface TelegramStatus { connected: boolean; paired: boolean; botName: string; pairingCode: string; error: string }

interface Tab {
  id: string;
  label: string;
  icon: string;
  render: (view: HTMLElement) => Promise<void> | void;
}

const TABS: Tab[] = [
  { id: "general", label: "General", icon: TABLER.settings, render: renderGeneral },
  { id: "folders", label: "Carpetas", icon: TABLER.folder, render: renderFolders },
  { id: "connections", label: "Conexiones", icon: SETTINGS_ICONS.plugConnected, render: renderConnections },
  { id: "usage", label: "Uso", icon: SETTINGS_ICONS.chartBar, render: renderUsage },
  { id: "agents", label: "Agentes", icon: SETTINGS_ICONS.users, render: async (view) => { const on = await renderAgents(view); if (current === "agents") onCoreEvent = on; } },
];

const nav = document.getElementById("tabs")!;
const page = document.getElementById("page")!;
/** What the open tab wants to hear from the core (events reach every window as "core-event"). */
let onCoreEvent: ((e: CoreEvent) => void) | null = null;
let current = "";

function show(id: string) {
  const tab = TABS.find((t) => t.id === id) ?? TABS[0]!;
  current = tab.id;
  onCoreEvent = null;
  for (const b of nav.querySelectorAll<HTMLButtonElement>("[role=tab]")) {
    const on = b.dataset.tab === tab.id;
    b.setAttribute("aria-selected", String(on));
    b.tabIndex = on ? 0 : -1;
  }
  // A fresh view per visit: a slow answer for a tab already left lands in a detached node, never on the new tab.
  const view = h("div", { class: "view", id: `view-${tab.id}` });
  page.replaceChildren(view);
  page.setAttribute("aria-labelledby", `tab-${tab.id}`);
  page.scrollTop = 0;
  void tab.render(view);
}

nav.append(...TABS.map((t) => {
  const b = h("button", { type: "button", role: "tab", id: `tab-${t.id}`, "aria-controls": "page", "data-tab": t.id }, icon(t.icon), h("span", { text: t.label }));
  b.addEventListener("click", () => show(t.id));
  return b;
}));
nav.addEventListener("keydown", (e) => {
  const step = e.key === "ArrowDown" ? 1 : e.key === "ArrowUp" ? -1 : 0;
  if (!step) return;
  e.preventDefault();
  const i = (TABS.findIndex((t) => t.id === current) + step + TABS.length) % TABS.length;
  show(TABS[i]!.id);
  nav.querySelector<HTMLButtonElement>(`#tab-${TABS[i]!.id}`)?.focus();
});

void listen<CoreEvent>("core-event", ({ payload }) => onCoreEvent?.(payload));

// MARK: General

function renderGeneral(view: HTMLElement) {
  view.append(
    header("General"),
    section("Buddy",
      settingRow("pet.wander", "Pasear por la pantalla", "Solo cuando no usas el teclado ni el ratón, y nunca con el chat abierto.")).el,
    renderModels(),
    section("Agentes",
      settingRow("commands.enabled", "Permitir que ejecuten comandos", "Siempre con tu clic: cada comando sale en la barra con Permitir o Rechazar.")).el,
    renderAlwaysRules(),
  );
  void renderBriefing(view);
}

interface AlwaysRule { agent: string; prefix: string; addedAt: number }
const AGENT_NAMES: Record<string, string> = { buddy: "Buddy", claude: "Claude Code", codex: "Codex", antigravity: "Gemini (Antigravity)" };

/** The commands allowed for good from a card («Permitir siempre»); each can be removed. */
function renderAlwaysRules(): HTMLElement {
  const { el, card } = section("Comandos permitidos siempre");
  el.append(h("p", { class: "muted small", text: "Solo comandos simples (sin «|», «;», «&&» ni redirecciones) y nunca rm, sudo, curl o parecidos." }));
  async function draw() {
    const rules = await invoke<AlwaysRule[]>("always_rules").catch(() => [] as AlwaysRule[]);
    if (!rules.length) { card.replaceChildren(emptyRow("Ninguno", "En la tarjeta de un comando, «Permitir siempre» lo añade aquí.")); return; }
    card.replaceChildren(...rules.map((r) => row(h("code", { class: "row-title", text: `${r.prefix} …` }), AGENT_NAMES[r.agent] ?? r.agent,
      button("Quitar", () => void invoke("remove_always_rule", { agent: r.agent, prefix: r.prefix }).then(draw), "secondary", "Volver a preguntar por este comando"))));
  }
  void draw();
  return el;
}

// MARK: Carpetas

/** The path with its last folder always visible; the start is what gets cut when it is long. */
interface ModelOption { id: string; name: string; provider: string }
interface TierChoice { tier: string; label: string; model: string; effort: string }
interface RouterConfig { mode: string; tiers: TierChoice[]; models: ModelOption[] }

const EFFORTS: [string, string][] = [["low", "Bajo"], ["medium", "Medio"], ["high", "Alto"]];
const TIER_DETAILS: Record<string, string> = {
  light: "Saludos y charla corta.",
  normal: "La mayoría de preguntas.",
  deep: "Análisis, comparaciones, planes, mensajes largos.",
  code: "Código, errores y archivos de programación.",
  work: "Word, Excel, presentaciones, informes y archivos.",
  math: "Cálculos, ecuaciones, estadística y demostraciones.",
};

/** A native <select> with (value, label) options. */
function select(options: [string, string][], value: string, label: string, onChange: (v: string) => void): HTMLSelectElement {
  const el = h("select", { class: "select", "aria-label": label },
    ...options.map(([v, text]) => h("option", { value: v, text, selected: v === value })));
  el.addEventListener("change", () => onChange(el.value));
  return el;
}

/** Which model answers Buddy: one fixed, or the router choosing by what is asked (rules, no tokens spent). */
function renderModels(): HTMLElement {
  const { el, card } = section("Modelo de Buddy");
  el.append(h("p", { class: "muted small", text: "En un mensaje puedes elegir tú: «con opus», «usa gpt», «con sonnet». Los especialistas como PARLEY usan su propio modelo." }));
  const preview = h("div", { class: "row-detail mono", role: "status" });
  const sample = h("input", { class: "text", type: "text", placeholder: "Escribe un pedido para ver qué modelo usaría", "aria-label": "Probar el router" });
  sample.addEventListener("input", async () => {
    preview.textContent = sample.value.trim() ? await invoke<string>("router_preview", { text: sample.value }).catch(() => "") : "";
  });

  async function draw() {
    const config = await invoke<RouterConfig>("router_config").catch(() => null);
    if (!config) { card.replaceChildren(emptyRow("No disponible")); return; }
    const models: [string, string][] = config.models.map((m) => [m.id, m.name]);
    const mode = select([["auto", "Automático (según lo que pidas)"], ...models], config.mode, "Modelo",
      (v) => void invoke("set_router_mode", { mode: v }).then(draw));
    const rows = [row("Modelo", null, mode)];
    if (config.mode === "auto") {
      for (const t of config.tiers) {
        const save = (model: string, effort: string) => void invoke("set_router_tier", { tier: t.tier, model, effort }).then(draw);
        rows.push(row(t.label, TIER_DETAILS[t.tier] ?? "",
          select(models, t.model, `Modelo para ${t.label}`, (v) => save(v, t.effort)),
          select(EFFORTS, t.effort, `Esfuerzo para ${t.label}`, (v) => save(t.model, v))));
      }
    }
    rows.push(h("div", { class: "field" }, sample, preview));
    card.replaceChildren(...rows);
  }

  void draw();
  return el;
}

function pathLabel(path: string): HTMLElement {
  const cut = Math.max(path.lastIndexOf("\\", path.length - 2), path.lastIndexOf("/", path.length - 2)) + 1;
  return h("span", { class: "path", title: path }, h("span", { class: "head", text: path.slice(0, cut) }), h("span", { class: "tail", text: path.slice(cut) }));
}

async function renderFolders(view: HTMLElement) {
  let folders: AuthorizedFolder[] = [];
  let selected: string | null = null;
  const list = h("div", { class: "list", role: "listbox", "aria-label": "Carpetas autorizadas" });
  const error = h("span", { class: "error", role: "alert" });
  const remove = iconButton(SETTINGS_ICONS.minus, "Quitar la carpeta elegida", () => {
    if (!selected) return;
    void invoke<AuthorizedFolder[]>("remove_folder", { path: selected }).then((f) => { selected = null; set(f); }, fail);
  });
  const add = iconButton(SETTINGS_ICONS.plus, "Añadir una carpeta", () => {
    void invoke<AuthorizedFolder[]>("pick_folder").then(set, fail);
  });

  function fail(e: unknown) { error.textContent = errorText(e); }
  function set(next: AuthorizedFolder[]) { folders = next; error.textContent = ""; draw(); }

  function draw() {
    remove.disabled = !selected || !folders.some((f) => f.path === selected);
    if (!folders.length) {
      list.replaceChildren(emptyRow("Sin carpetas", "Añade las carpetas que Buddy puede usar."));
      return;
    }
    list.replaceChildren(...folders.map((f) => {
      const check = h("input", { type: "checkbox", "aria-label": `Puede editar en ${f.path}` });
      check.checked = f.canEdit;
      check.addEventListener("change", () => {
        void invoke<AuthorizedFolder[]>("set_folder_edit", { path: f.path, canEdit: check.checked }).then(set, (e) => { check.checked = f.canEdit; fail(e); });
      });
      const isSelected = f.path === selected;
      const item = h("div", { class: `row folder${isSelected ? " selected" : ""}`, role: "option", "aria-selected": String(isSelected), tabindex: "0" },
        h("span", { class: "folder-icon" }, icon(TABLER.folder)),
        pathLabel(f.path),
        h("label", { class: "check" }, check, h("span", { text: "Puede editar" })));
      const pick = () => { selected = f.path; draw(); list.querySelector<HTMLElement>(".row.selected")?.focus(); };
      item.addEventListener("click", (e) => { if (!(e.target as Element).closest("label")) pick(); });
      item.addEventListener("keydown", (e) => {
        if (e.target !== item) return;
        if (e.key === " " || e.key === "Enter") { e.preventDefault(); pick(); }
        if (e.key === "Delete" || e.key === "Backspace") { selected = f.path; remove.click(); }
      });
      return item;
    }));
  }

  const { el } = section(null, list, h("div", { class: "toolbar" }, add, remove, error));
  view.append(header("Carpetas", "Buddy y sus agentes solo pueden leer (y, si lo marcas, editar) dentro de estas carpetas."), el);
  draw();
  await invoke<AuthorizedFolder[]>("folders").then(set, fail);
}

// MARK: Conexiones

async function renderConnections(view: HTMLElement) {
  const message = h("p", { class: "muted small", role: "status" });
  const { el, card } = section("Avisos de tus sesiones");
  const dialog = h("dialog", { class: "confirm", "aria-labelledby": "confirm-title" });
  const youtube = renderYouTube();
  const phone = renderPhone();
  onCoreEvent = event => {
    if (event.type === "youTubeChanged") void youtube.refresh();
    if (event.type === "remoteChanged") void phone.refresh();
  };
  view.append(header("Conexiones"), el, message, dialog, youtube.element, renderSpotify(), renderTelegram(), phone.element, renderTelegramAccount(), renderParleyOdds(), renderConnectors());

  async function draw() {
    const status = await invoke<HookStatusInfo[]>("hooks_status").catch(() => [] as HookStatusInfo[]);
    if (!status.length) { card.replaceChildren(emptyRow("Nada que conectar")); return; }
    card.replaceChildren(...status.map((item) => {
      const tile = h("span", { class: "mark-tile" }, providerMark(item.agent, 14));
      const state = !item.available ? "No instalado" : item.installed ? "Conectado: sus avisos llegan a la barra" : "Sin conectar";
      const r = row(item.name, state, item.available ? button(item.installed ? "Desconectar" : "Conectar…", () => void ask(item)) : null);
      r.prepend(tile);
      r.classList.add("with-lead");
      return r;
    }));
  }

  /** Shows the exact change before writing it (a dated backup is kept), like the Mac's alert. */
  async function ask(item: HookStatusInfo) {
    const install = !item.installed;
    let preview: HookPreview;
    try {
      preview = await invoke<HookPreview>("hooks_preview", { agent: item.agent, install });
    } catch (e) {
      message.textContent = `No se pudo: ${errorText(e)}`;
      return;
    }
    const diff = h("pre", { class: "diff", tabindex: "0", "aria-label": "Cambios" },
      ...preview.diff.split("\n").map((line) =>
        h("span", { class: line.startsWith("+") ? "add" : line.startsWith("-") ? "del" : "", text: `${line}\n` })));
    const ok = button(install ? "Conectar" : "Desconectar", () => dialog.close("ok"), "primary");
    const cancel = button("Cancelar", () => dialog.close("cancel"));
    dialog.replaceChildren(
      h("h3", { id: "confirm-title", text: install ? `¿Conectar ${item.name} con Buddy?` : `¿Desconectar ${item.name}?` }),
      h("p", { class: "muted", text: `${install ? "Buddy añadirá sus avisos a " : "Buddy quitará sus avisos de "}${preview.path}. Antes guarda una copia; no toca nada más.` }),
      diff,
      h("div", { class: "actions" }, cancel, ok));
    dialog.returnValue = "";
    dialog.showModal();
    cancel.focus();
    const answer = await new Promise<string>((resolve) => dialog.addEventListener("close", () => resolve(dialog.returnValue), { once: true }));
    if (answer !== "ok") return;
    try {
      const backup = await invoke<string>("hooks_write", { agent: item.agent, install, fingerprint: preview.fingerprint });
      message.textContent = backup ? `Hecho. Copia en ${backup}` : "Hecho.";
    } catch (e) {
      message.textContent = `No se pudo: ${errorText(e)}`;
    }
    await draw();
  }

  await draw();
}

/** Spotify's search for Buddy: the user's own app (Client ID in settings, Client Secret only in Credential Manager). */
function renderSpotify(): HTMLElement {
  const { el, card } = section("Spotify");
  const note = h("p", { class: "muted small", text: "Spotify exige Premium en la cuenta dueña de la app. Reproducir no lo necesita: Buddy usa la app de Spotify de tu PC." });
  el.append(note);

  async function draw() {
    const id = await invoke<string>("spotify_client_id").catch(() => "");
    if (id) {
      card.replaceChildren(row("Conectado", `App ${id.slice(0, 6)}… · el secreto está en el Administrador de credenciales`,
        button("Desconectar", () => void invoke("spotify_disconnect").then(draw))));
      return;
    }
    const clientId = h("input", { class: "text", type: "text", placeholder: "Client ID", "aria-label": "Client ID", autocomplete: "off", spellcheck: "false" });
    const secret = h("input", { class: "text", type: "password", placeholder: "Client Secret", "aria-label": "Client Secret", autocomplete: "off" });
    const error = h("p", { class: "error small", role: "alert" });
    const connect = button("Conectar", async () => {
      connect.disabled = true;
      error.textContent = "";
      try {
        await invoke("spotify_connect", { clientId: clientId.value, clientSecret: secret.value });
        await draw();
      } catch (e) {
        error.textContent = errorText(e);
        connect.disabled = false;
      }
    }, "primary");
    card.replaceChildren(
      h("div", { class: "field" },
        h("p", { class: "muted small", text: "Para que Buddy busque y ponga música dentro de Spotify (sin buscar en la web):" }),
        h("p", { class: "muted small", text: "1. Abre el panel de desarrolladores y crea una app (cualquier nombre; en «Redirect URI» pon http://127.0.0.1:8888, no se usa)." }),
        h("p", { class: "muted small", text: "2. Marca «Web API», guarda y copia aquí su Client ID y su Client Secret." }),
        h("div", {}, button("Abrir el panel de Spotify", () => void invoke("open_url", { url: "https://developer.spotify.com/dashboard" }))),
        clientId, secret,
        h("div", { class: "actions" }, error, connect)));
  }

  void draw();
  return el;
}

/** Telegram: the user's own bot (token only in Credential Manager), paired with one chat by `/start <code>`.
 *  PARLEY answers that chat; nothing else is heard. The core does all of it. */
function renderTelegram(): HTMLElement {
  const { el, card } = section("Telegram");
  el.append(h("p", { class: "muted small", text: "El token se guarda solo en el Administrador de credenciales. Buddy atiende únicamente al chat vinculado, y desde Telegram PARLEY no ejecuta comandos ni cambia archivos." }));
  let note = "";

  async function draw() {
    const status = await invoke<TelegramStatus>("telegram_status").catch(() => null);
    if (!status) { card.replaceChildren(emptyRow("No se pudo leer el estado de Telegram.")); return; }
    const problem = status.error ? h("p", { class: "error small", role: "alert", text: status.error }) : null;
    const disconnect = button("Desconectar", () => void invoke("telegram_disconnect").then(() => { note = ""; return draw(); }));
    const newCode = (label: string) => button(label, () => void invoke("telegram_new_pairing_code").then(() => { note = ""; return draw(); }));
    if (!status.connected) {
      const token = h("input", { class: "text", type: "password", placeholder: "Token del bot (123456789:AA…)", "aria-label": "Token del bot", autocomplete: "off", spellcheck: "false" });
      const error = h("p", { class: "error small", role: "alert" });
      const connect = button("Conectar", async () => {
        connect.disabled = true;
        error.textContent = "";
        try {
          await invoke("telegram_connect", { token: token.value });
          await draw();
        } catch (e) {
          error.textContent = errorText(e);
          connect.disabled = false;
        }
      }, "primary");
      card.replaceChildren(
        h("div", { class: "field" },
          h("p", { class: "muted small", text: "Para hablar con PARLEY desde Telegram y recibir ahí sus picks:" }),
          h("p", { class: "muted small", text: "1. Abre @BotFather, envíale /newbot y elige un nombre para tu bot." }),
          h("p", { class: "muted small", text: "2. Copia el token que te da y pégalo aquí." }),
          h("div", {}, button("Abrir @BotFather", () => void invoke("open_url", { url: "https://t.me/BotFather" }))),
          token,
          h("div", { class: "actions" }, error, connect)),
        ...(problem ? [problem] : []));
      return;
    }
    if (!status.paired) {
      const bot = status.botName.replace(/^@/, "");
      const link = `https://t.me/${encodeURIComponent(bot)}?start=${encodeURIComponent(status.pairingCode)}`;
      card.replaceChildren(
        row(`Conectado con ${status.botName}`, "Falta vincular tu chat", disconnect),
        h("div", { class: "field" },
          h("p", { class: "muted small", text: `Envía este mensaje a ${status.botName} desde tu Telegram:` }),
          h("p", { class: "pairing-code", text: `/start ${status.pairingCode}` }),
          h("p", { class: "muted small", text: "El código vale 15 minutos. Solo el chat que lo envíe podrá hablar con Buddy." }),
          h("div", { class: "actions" },
            button(`Abrir ${status.botName} en Telegram`, () => void invoke("open_url", { url: link })),
            newCode("Nuevo código"))),
        ...(problem ? [problem] : []));
      return;
    }
    const test = button("Enviar prueba", async () => {
      test.disabled = true;
      try {
        await invoke("telegram_send", { text: "¡Hola! Soy Buddy. Así te llegarán los mensajes y los picks de PARLEY." });
        note = "Enviado. Míralo en Telegram.";
      } catch (e) {
        note = errorText(e);
      }
      await draw();
    });
    card.replaceChildren(
      row(`Conectado con ${status.botName}`, "Tu chat está vinculado: lo que le escribas lo responde PARLEY.", test, disconnect),
      h("div", { class: "field" }, h("div", { class: "actions" }, h("p", { class: "muted small", role: "status", text: note }), newCode("Cambiar de chat"))),
      ...(problem ? [problem] : []));
  }

  onCoreEvent = (e) => {
    if (e.type === "telegramChanged") void draw();
  };
  void draw();
  return el;
}

// MARK: Uso

async function renderUsage(view: HTMLElement) {
  void invoke("refresh_usage").catch(() => {});
  const plans = section("Tus planes");
  const tokens = section("Tokens de los últimos 7 días");
  const calendar = h("section", { class: "group" });
  view.append(header("Uso"), calendar, plans.el, tokens.el);

  async function drawCalendar() {
    try {
      const days = await invoke<TokenDay[]>("token_activity", { days: 371 });
      calendar.replaceChildren(usageCalendar(days));
    } catch {
      calendar.replaceChildren(emptyRow("No se pudo cargar la actividad."));
    }
  }

  async function drawPlans() {
    const usage = await invoke<ProviderUsage[]>("usage").catch(() => [] as ProviderUsage[]);
    const rows = usage.flatMap((plan) => plan.windows.map((w) => {
      const pct = Math.min(Math.max(Math.round(w.usedPct), 0), 100);
      const level = pct >= 90 ? "high" : pct >= 70 ? "warn" : "";
      const reset = w.resetsAt ? `Se reinicia ${new Date(w.resetsAt * 1000).toLocaleString("es", { weekday: "long", hour: "2-digit", minute: "2-digit" })}` : null;
      const title = h("div", { class: "row-title with-mark" }, providerMark(plan.provider, 12), h("span", { text: `${plan.name} · ${w.label}` }));
      const meter = h("div", { class: "meter", role: "progressbar", "aria-valuemin": "0", "aria-valuemax": "100", "aria-valuenow": String(pct), "aria-label": `${plan.name} · ${w.label}` },
        h("i", { class: level, style: `width:${pct}%` }));
      return row(title, reset, meter, h("span", { class: "pct", text: `${pct} %` }));
    }));
    if (!usage.some(plan => plan.provider === "antigravity" && plan.windows.length)) {
      rows.push(row("Gemini", "Cuotas no disponibles"));
    }
    plans.card.replaceChildren(...rows);
  }

  async function drawTokens() {
    const report = await invoke<TokenReport[]>("token_report", { days: 7 }).catch(() => [] as TokenReport[]);
    tokens.card.replaceChildren(...(report.length ? report.map((r) => {
      const figures = h("span", {
        class: "figures",
        text: `${k(r.input + r.cached)} entrada · ${k(r.output)} salida`,
        title: r.costUsd > 0 ? `Equivaldría a ${r.costUsd.toFixed(2)} US$ en la API (tu plan no paga extra)` : undefined,
      });
      return row(r.feature, `${r.turns} turnos · ${r.provider === "codex" ? "Codex" : r.provider === "antigravity" ? "Gemini" : "Claude"}`, figures);
    }) : [emptyRow("Todavía no hay turnos medidos.")]));
  }

  onCoreEvent = (e) => {
    if (["usageChanged", "chatDone", "chatFailed"].includes(e.type)) {
      void Promise.all([drawPlans(), drawTokens(), drawCalendar()]);
    }
  };
  await Promise.all([drawPlans(), drawTokens(), drawCalendar()]);
}

// MARK: Mensajitos

let searching: number | undefined;

async function renderBriefing(view: HTMLElement) {
  const topics = h("textarea", { rows: 3, "aria-label": "Temas", spellcheck: "false" });
  const saved = h("span", { class: "muted small", role: "status" });
  const save = async (quiet = false) => {
    try {
      topics.value = await invoke<string>("set_briefing_topics", { topics: topics.value });
      if (!quiet) saved.textContent = "Guardado.";
    } catch (e) {
      saved.textContent = `No se pudo: ${errorText(e)}`;
    }
  };
  topics.addEventListener("keydown", (e) => { if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) { e.preventDefault(); void save(); } });
  topics.addEventListener("input", () => { saved.textContent = ""; });

  const today = section("Hoy");
  const list = h("div", { class: "list" });
  const now = button("Buscar ahora", () => {
    void save(true).then(() => invoke("briefing_now"));
    // With nothing new the core stays quiet: stop saying «Buscando…» after a while.
    window.clearTimeout(searching);
    searching = window.setTimeout(() => { searching = undefined; void draw(); }, 120_000);
    void draw();
  }, "secondary", "Una búsqueda ahora, aunque no sea la hora");
  today.card.append(list, h("div", { class: "toolbar end" }, now));

  async function draw() {
    const items = await invoke<BriefingItem[]>("briefing").catch(() => [] as BriefingItem[]);
    if (!items.length) {
      list.replaceChildren(emptyRow(searching !== undefined ? "Buscando… aparecerán aquí y en la barra." : "Todavía nada hoy."));
      return;
    }
    list.replaceChildren(...items.map((item) => {
      const web = item.url && /^https?:\/\//i.test(item.url) ? item.url : null;
      const r = row(item.text, item.topic, web ? iconButton(SETTINGS_ICONS.externalLink, "Abrir la fuente", () => void invoke("open_url", { url: web })) : null);
      r.classList.add("news");
      return r;
    }));
  }

  view.append(
    section("Novedades del día", settingRow("briefing.enabled", "Activar novedades",
      "A las 8, 16 y 19 h Buddy busca lo nuevo con el modelo más barato (3 búsquedas como mucho). Si no hay nada nuevo, no dice nada. Al empezar otro día se borran las noticias anteriores de todos los paneles.")).el,
    section("Qué buscar",
      h("div", { class: "field" }, topics,
        h("div", { class: "footer-line" },
          h("span", { class: "muted small", text: "Escribe temas separados por punto y coma. Vacío vuelve a los de siempre." }),
          button("Guardar", () => void save(), "primary")),
        saved)).el,
    today.el,
  );
  onCoreEvent = (e) => {
    if (e.type !== "briefingReady") return;
    window.clearTimeout(searching);
    searching = undefined;
    void draw();
  };
  topics.value = await invoke<string>("briefing_topics").catch(() => "");
  await draw();
}

show(TABS[0]!.id);
import "../shortcuts";
