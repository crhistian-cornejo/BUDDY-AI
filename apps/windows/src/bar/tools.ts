import { invoke } from "@tauri-apps/api/core";
import { h, svg } from "../chat/dom";
import { TABLER } from "../chat/tabler";

export interface ShelfFile { path: string; name: string; available: boolean }
export interface SavedClip { id: string; text: string; savedAt: number }
export interface NotchTools { files: ShelfFile[]; clips: SavedClip[]; batteryEnabled: boolean; calendarEnabled: boolean; clipboardEnabled: boolean; calendarPath: string | null }
interface Battery { percent: number | null; charging: boolean; pluggedIn: boolean }
interface Appointment { title: string; start: number; end: number; allDay: boolean; source: string; location: string; url: string | null }
type Tab = "home" | "files" | "utilities";
const $ = (id: string) => document.getElementById(id)!;
const glyph = (path: string) => svg(path, 14, { fill: "none", stroke: "currentColor", "stroke-width": "1.75", "stroke-linecap": "round", "stroke-linejoin": "round" });
const button = (label: string, action: () => void, primary = false, tip = label) => h("button", { class: `btn ${primary ? "primary" : ""}`, type: "button", title: tip, onclick: action }, label);
const smallButton = (path: string, label: string, action: () => void, disabled = false) => h("button", { class: "icon-btn", type: "button", title: label, "aria-label": label, disabled, onclick: action }, glyph(path));

/** Locally saved tools. Reading the clipboard always starts with an explicit button press. */
export class NotchToolPanel {
  tab: Tab = "home";
  private state: NotchTools = { files: [], clips: [], batteryEnabled: true, calendarEnabled: true, clipboardEnabled: true, calendarPath: null };
  private battery: Battery | null = null;
  private appointment: Appointment | null = null;
  private calendarError = "";
  private active = false;
  private interval = 0;
  private generation = 0;
  private drawKey = "";
  private pending = Promise.resolve();
  private widgets = h("select", { class: "widget-picker", "aria-label": "Elegir widgets", title: "Mostrar u ocultar widgets" });
  private tabs: Record<Tab, HTMLButtonElement>;

  constructor(private changed: () => void) {
    this.tabs = Object.fromEntries((["home", "files", "utilities"] as Tab[]).map((tab) => [tab,
      h("button", { class: "tool-tab", type: "button", onclick: () => this.select(tab) })])) as typeof this.tabs;
    this.widgets.addEventListener("change", () => {
      const widget = this.widgets.value;
      const enabled = !this.state[`${widget}Enabled` as "batteryEnabled" | "calendarEnabled" | "clipboardEnabled"];
      this.mutate("notch_widget", { widget, enabled });
      this.widgets.value = "";
    });
    $("tool-tabs").replaceChildren(...Object.values(this.tabs));
    $("widget-controls").replaceChildren(this.widgets);
    this.mutate("notch_tools");
  }

  select(tab: Tab) {
    this.tab = tab;
    if (tab === "files") this.mutate("notch_tools");
    this.changed();
  }

  addFiles(paths: string[]) {
    this.tab = "files";
    this.mutate("notch_add_files", { paths });
    this.changed();
  }

  private message(text: string) {
    $("tool-message").textContent = text;
    $("tool-message").hidden = !text;
  }

  private run(action: () => Promise<void>) {
    this.pending = this.pending.then(async () => {
      this.message("");
      try { await action(); } catch (error) { this.message(String(error)); }
      this.changed();
    });
  }

  private mutate(command: string, args?: Record<string, unknown>) {
    this.run(async () => {
      this.state = await invoke<NotchTools>(command, args);
      this.generation++;
      if (this.active) await this.refresh();
    });
  }

  private action(command: string, args?: Record<string, unknown>, success = "") {
    this.run(async () => { await invoke(command, args); this.message(success); });
  }

  draw(open: boolean) {
    for (const [tab, label] of [["home", "Inicio"], ["files", `Archivos${this.state.files.length ? ` · ${this.state.files.length}` : ""}`], ["utilities", "Utilidades"]] as const) {
      const path = tab === "home" ? "M5 12l-2 0l9 -9l9 9l-2 0 M5 12v8a1 1 0 0 0 1 1h4v-7h4v7h4a1 1 0 0 0 1 -1v-8"
        : tab === "files" ? "M12 5v14 M5 12h14" : "M3 3h7v7h-7z M14 3h7v7h-7z M3 14h7v7h-7z M14 14h7v7h-7z";
      if (!this.tabs[tab].firstChild) this.tabs[tab].append(glyph(path));
      this.tabs[tab].title = label;
      this.tabs[tab].setAttribute("aria-label", label);
      this.tabs[tab].setAttribute("aria-pressed", String(this.tab === tab));
    }
    this.widgets.hidden = this.tab !== "utilities";
    $("widget-controls").hidden = this.tab !== "utilities";
    const options = [["battery", "Batería"], ["calendar", "Próxima cita"], ["clipboard", "Portapapeles"]] as const;
    // Keep the select and tab buttons alive when background session events arrive.
    if (document.activeElement !== this.widgets) this.widgets.replaceChildren(h("option", { value: "", text: "Widgets…" }),
      ...options.map(([id, title]) => h("option", { value: id, text: `${this.state[`${id}Enabled`] ? "✓" : "○"} ${title}` })));
    $("home-tools").hidden = this.tab !== "home";
    $("files-tools").hidden = this.tab !== "files";
    $("utility-tools").hidden = this.tab !== "utilities";
    const active = open && this.tab === "utilities";
    if (active !== this.active) {
      this.active = active; this.generation++;
      window.clearInterval(this.interval);
      if (active) {
        void this.refresh();
        this.interval = window.setInterval(() => void this.refresh(), 30_000);
      }
    }
    const key = JSON.stringify([this.tab, this.state, this.battery, this.appointment, this.calendarError]);
    if (key === this.drawKey) return;
    this.drawKey = key;
    if (this.tab === "files") this.drawShelf();
    if (this.tab === "utilities") this.drawUtilities();
  }

  private async refresh() {
    const generation = this.generation;
    const [battery, calendar] = await Promise.allSettled([
      this.state.batteryEnabled ? invoke<Battery | null>("notch_battery") : Promise.resolve(null),
      this.state.calendarEnabled && this.state.calendarPath ? invoke<Appointment | null>("notch_calendar") : Promise.resolve(null),
    ]);
    if (!this.active || generation !== this.generation) return;
    this.battery = battery.status === "fulfilled" ? battery.value : null;
    this.appointment = calendar.status === "fulfilled" ? calendar.value : null;
    this.calendarError = calendar.status === "rejected" ? "No se pudo leer la agenda. Vuelve a elegir el archivo .ics." : "";
    this.changed();
  }

  private drawShelf() {
    const files = this.state.files;
    const rows = files.map((file) => h("div", { class: "shelf-row" },
      h("button", { class: "file-reveal", type: "button", title: file.path, disabled: !file.available,
        onclick: () => this.action("reveal_path", { path: file.path }) }, glyph(TABLER.fileText),
        h("span", {}, h("strong", { text: file.name }), h("small", { text: file.available ? file.path : "Archivo no disponible" }))),
      smallButton(TABLER.x, `Retirar ${file.name} de la bandeja; el archivo permanece en su carpeta`, () => this.mutate("notch_remove_file", { path: file.path }))));
    const available = files.filter((f) => f.available).map((f) => f.path);
    const give = button("Dárselo a Buddy", () => this.action("give_files", { paths: available }), true, "Abrir el chat con los archivos de la bandeja");
    const copy = button("Copiar rutas", () => this.action("notch_copy_paths", undefined, "Rutas copiadas."), false, "Copiar las rutas de los archivos");
    give.disabled = copy.disabled = !available.length;
    $("files-tools").replaceChildren(
      h("div", { class: "tool-head" }, h("strong", { text: "Tu bandeja" }), h("small", { class: "muted", text: `${files.length}/32` }),
        h("span", { class: "spacer" }), button("Añadir…", () => this.mutate("notch_pick_files"), false, "Añadir archivos a la bandeja")),
      h("div", { class: "shelf-list" }, ...(rows.length ? rows : [h("div", { class: "shelf-empty" }, glyph(TABLER.folder),
        h("span", { text: "Suelta archivos aquí o pulsa Añadir." }), h("small", { class: "muted", text: "Se conservan al cerrar el notch." }))])),
      h("div", { class: "row-btns" }, give, copy));
  }

  private drawUtilities() {
    const cards: HTMLElement[] = [];
    if (this.state.batteryEnabled) cards.push(h("div", { class: "tile utility-card" }, h("h2", {}, "Batería"),
      h("strong", { class: "battery-percent", text: this.battery ? `${this.battery.percent ?? "—"}${this.battery.percent === null ? "" : " %"}${this.battery.pluggedIn ? " ⚡" : ""}` : "Equipo sin batería" }),
      h("small", { class: "muted", text: this.battery ? this.battery.charging ? "Cargando" : this.battery.pluggedIn ? "Conectado a corriente" : "Usando batería" : "Datos del sistema" })));
    if (this.state.calendarEnabled) {
      const event = this.appointment;
      const when = event ? new Date(event.start * 1000).toLocaleString("es", { weekday: "short", day: "numeric", month: "short", ...(event.allDay ? {} : { hour: "2-digit", minute: "2-digit" }) }) + (event.allDay ? " · todo el día" : "") : "";
      const settings = h("select", { class: "calendar-picker", "aria-label": "Fuente de la agenda", title: "Elegir o desconectar agenda" },
        h("option", { value: "", text: "···" }), h("option", { value: "pick", text: "Elegir agenda .ics…" }),
        this.state.calendarPath ? h("option", { value: "clear", text: "Desconectar agenda" }) : null);
      settings.addEventListener("change", () => { this.mutate(settings.value === "pick" ? "notch_pick_calendar" : "notch_clear_calendar"); settings.value = ""; });
      cards.push(h("div", { class: "tile utility-card" }, h("div", { class: "tool-head" }, h("h2", {}, "Próxima cita"), h("span", { class: "spacer" }), settings),
        event ? h("button", { class: "appointment", type: "button", title: `${event.title} · ${when} · ${event.source}${event.location ? ` · ${event.location}` : ""}\nAbrir enlace o evento en tu aplicación de calendario`,
          onclick: () => this.action("notch_open_appointment") }, h("strong", { text: event.title || "Sin título" }), h("small", { class: "event-when", text: when }), h("small", { class: "muted", text: event.source }))
          : this.state.calendarPath ? h("span", { class: "muted", text: this.calendarError || "Sin próximas citas" })
            : h("div", {}, h("small", { class: "muted", text: "Agenda local (.ics)" }), button("Elegir agenda…", () => this.mutate("notch_pick_calendar")))));
    }
    const body: Node[] = cards.length ? [h("div", { class: "utility-top" }, ...cards)] : [];
    if (this.state.clipboardEnabled) body.push(h("div", { class: "tile clipboard-card" },
      h("div", { class: "tool-head" }, h("h2", {}, "Portapapeles"), h("span", { class: "spacer" }),
        h("button", { class: "pill", type: "button", title: "Guardar el texto que tú has copiado; no se guarda nada automáticamente", onclick: () => this.mutate("notch_save_clip") }, "Guardar actual")),
      h("div", { class: "clip-list" }, ...(this.state.clips.length ? this.state.clips.map((clip) => h("div", { class: "clip-row" },
        h("span", { class: "clip-text", text: clip.text, title: clip.text.slice(0, 2000) }),
        smallButton("M9 5h10v14h-10z M5 9h-2v12h10v-2", "Copiar texto", () => this.action("notch_copy_clip", { id: clip.id }, "Texto copiado.")),
        smallButton("M3 3h18v14h-12l-6 4z", "Llevar al chat sin enviarlo", () => this.action("notch_give_clip", { id: clip.id })),
        smallButton(TABLER.x, "Quitar texto guardado", () => this.mutate("notch_remove_clip", { id: clip.id })))) :
        [h("span", { class: "muted", text: "Guarda textos o enlaces para copiarlos o llevarlos al chat." })]))));
    if (!body.length) body.push(h("div", { class: "widgets-empty muted", text: "Elige un widget en el menú Widgets." }));
    $("utility-tools").replaceChildren(...body);
  }
}
