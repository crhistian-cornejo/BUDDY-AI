// Isolated UI fixture: never loads Buddy data or touches the real clipboard/files.
import { NotchToolPanel, type NotchTools } from "../src/bar/tools";
const tools: NotchTools = {
  files: [{ path: "C:\\Documentos\\Proyecto\\notas.txt", name: "notas.txt", available: true },
    { path: "C:\\Documentos\\Proyecto\\informe con un nombre bastante largo para revisar los espacios.pdf", name: "informe con un nombre bastante largo para revisar los espacios.pdf", available: true },
    { path: "C:\\Descargas\\archivo-movido.zip", name: "archivo-movido.zip", available: false }],
  clips: [{ id: "1", text: "Texto guardado de prueba, sin enviarlo automáticamente.", savedAt: 0 }, { id: "2", text: "https://github.com/Ebullioscopic/Atoll", savedAt: 0 }],
  batteryEnabled: true, calendarEnabled: true, clipboardEnabled: true, calendarPath: "C:\\Agenda\\trabajo.ics",
};
Object.assign(window, { __TAURI_INTERNALS__: { invoke: async (command: string, args: Record<string, any>) => {
  switch (command) {
    case "notch_battery": return { percent: 76, charging: true, pluggedIn: true };
    case "notch_calendar": return tools.calendarPath ? { title: "Revisión del diseño del notch con el equipo", start: Date.now() / 1000 + 3600, end: Date.now() / 1000 + 7200, allDay: false, location: "Sala de pruebas", source: "Trabajo · agenda de prueba", url: null } : null;
    case "notch_widget": tools[`${args.widget}Enabled` as "batteryEnabled"] = args.enabled; break;
    case "notch_remove_file": tools.files = tools.files.filter((f) => f.path !== args.path); break;
    case "notch_remove_clip": tools.clips = tools.clips.filter((c) => c.id !== args.id); break;
    case "notch_pick_files": tools.files.push({ path: "C:\\Prueba\\nuevo.txt", name: "nuevo.txt", available: true }); break;
    case "notch_save_clip": tools.clips.unshift({ id: String(Date.now()), text: "Texto añadido desde el portapapeles de prueba.", savedAt: 0 }); break;
    case "notch_clear_calendar": tools.calendarPath = null; break;
    case "notch_pick_calendar": tools.calendarPath = "C:\\Agenda\\trabajo.ics"; break;
  }
  return structuredClone(tools);
} } });
const panel = new NotchToolPanel(() => panel.draw(true));
panel.draw(true);
