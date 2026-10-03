# Fase 2: el notch (Mac) y la barra superior (Windows)

**Objetivo:** avisos sobrios de Claude Code y Codex (terminó, espera, error, pide permiso con Permitir/Rechazar), música, chats recientes y sesiones abiertas, en el notch del Mac y en una barra arriba en Windows, con la misma lógica en el núcleo.

## Piezas

1. **`buddy-hook` (relevo).**
   - Pequeño ejecutable que Claude Code y Codex lanzan en cada evento de hook.
   - Lee el JSON, lo etiqueta (`_agent`) y lo recorta.
   - Lo entrega a Buddy por un socket Unix (`<datos>/hooks.sock`, Mac) o una tubería con nombre (`\\.\pipe\buddy-<SID>`, Windows).
   - **Nunca bloquea al agente:** si Buddy no está abierto, sale sin decir nada.
   - Solo `PermissionRequest` espera respuesta, como mucho 110 s; sin respuesta, el agente pregunta en la terminal como siempre.
2. **`core/sessions`.**
   - Servidor local: solo el mismo usuario; 1 MB y 5 s por petición; como mucho 16 conexiones.
   - Estado de cada sesión (trabajando, esperando, terminó, error, cerrada).
   - Peticiones de permiso pendientes y su respuesta (`answer_approval`).
   - Preguntas de Codex: Codex no tiene un evento para «te pregunto algo»; su herramienta `request_user_input` llega como un `PreToolUse` cualquiera (la variante `_async` vuelve al instante y Codex sigue trabajando). El núcleo la reconoce, deja la sesión «esperando» hasta que el usuario responde (`UserPromptSubmit`, o el `PostToolUse` de la variante que espera) y envía la pregunta en `SessionUpdate.summary`; el notch abre la tarjeta con ella y un botón para ir a responder. Las aprobaciones de Codex solo llegan si su sesión pregunta antes de ejecutar: con «Acceso completo» (política `never`) Codex no pide permiso y no hay nada que mostrar.
   - Instalador que edita `~/.claude/settings.json` y `~/.codex/hooks.json`, siempre tras un clic: muestra el cambio, guarda una copia fechada, no toca hooks ajenos y se puede deshacer.
   - Eventos al bus: `SessionUpdate`, `ApprovalRequest`, `ApprovalClosed`. Con un permiso pendiente, Buddy muestra «pregunta».
3. **Mac (SwiftUI/AppKit), `Sources/Notch`.**
   - Isla negra con «orejas» que cuelga del notch. En pantallas sin notch, una píldora arriba al centro.
   - **En reposo:** mide exactamente el notch. Con una sesión activa salen las orejas (marca del agente y punto de estado); con un bloque de enfoque, los minutos que quedan.
   - **Avisos:** uno a la vez, 6 s cada uno; un permiso se queda hasta que respondes y adelanta a los demás.
   - **Al pasar el ratón, herramientas** (sin chats: el historial vive en el chat):
     - reproductor: carátula, título, artista, progreso con tiempos y controles (Spotify o Música por AppleScript, solo mientras está abierta);
     - **Agentes:** sesiones con su estado, o «Conectar»;
     - **Enfoque:** 25 o 50 min con anillo; avisa al terminar y Buddy celebra;
     - **Atajos:** hasta 8 apps, carpetas, archivos o webs; «+» abre el selector del sistema.
   - **«Suelta tus archivos»:** al arrastrar archivos al notch, se pueden dar a Buddy, compartir con la hoja del sistema (AirDrop, Mail…) o copiar su ruta.
   - **Entrada del ratón:** la ventana solo la recibe dentro de la forma; fuera, los clics pasan a lo que hay debajo.
4. **`core/usage`:** cuánto queda de cada plan (5 h, semana, mes).
   - Sale de lo que Claude y Codex ya dicen en cada turno.
   - Al abrir el notch se piden cifras frescas: Codex cada 5 min como mucho, sin modelo; Claude cada 30 min como mucho, con un turno de una palabra en Haiku.
   - Al cruzar el 95 % de una ventana, el notch avisa «Te queda N %».
5. **`core/tools`:** el temporizador de enfoque (un hilo que duerme hasta el final, sin «tic») y los atajos fijados, para las dos apps.
6. **Windows (Tauri), `bar.html`.**
   - La misma isla negra arriba al centro: píldora en reposo, tarjeta con un aviso y, al pasar el ratón, las mismas herramientas.
   - **Soltar archivos:** dárselos a Buddy, mostrarlos en el Explorador o copiar la ruta.
   - **Música:** «now playing» del sistema (Windows.Media.Control, portado de MIKA), solo mientras está abierta.
   - **Conectar:** diálogo nativo con el cambio antes de escribir.

## Se acepta cuando

- [ ] Una sesión de Claude Code que termina sale como «Claude Code terminó · proyecto» en el notch y se va sola.
- [ ] Un permiso de Claude Code se puede aceptar o rechazar desde el notch, y la sesión sigue según la respuesta.
- [ ] Sin Buddy abierto, Claude Code funciona exactamente igual (el relevo sale sin decir nada).
- [ ] La música se controla desde el notch o la barra.
- [ ] En reposo, ~0 % de CPU: nada sondea mientras no hay movimiento del ratón ni avisos.
