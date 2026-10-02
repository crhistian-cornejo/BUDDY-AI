# Buddy: plan maestro

Versión 2 · 2026-10-02 · Aprobado en lo general por el dueño, con los cambios de esta versión. Sustituye al borrador de MIKA (`MIKA/docs/superpowers/specs/2026-10-02-nueva-app-plan-maestro.md`).

## 1. Qué es Buddy

Un asistente personal que no obliga a tener un chat abierto todo el día. Tiene dos superficies, iguales en Mac y Windows: la mascota y el notch (en Windows, la barra superior). El usuario elige usar una o las dos.

### 1.1 La mascota: Buddy

- **Qué es.** Un personaje en pixel art de 8 bits, al estilo de Clawd (la mascota de Claude Code) y de las mascotas de Codex, dibujado en código como cuadrícula de píxeles. No usa imágenes ni los círculos con ojos de MIKA.
  - Podrá importar personajes en el formato de ChatGPT/Codex y en el de CoPet/Petdex.
- **En reposo.** Flota en una esquina por encima de las ventanas y casi no consume.
- **Al hacerle clic.** Muestra un compositor compacto a su lado, con «+» (adjuntar archivo, captura o carpeta), «Iniciar nuevo chat», micrófono y enviar.
- **Cuando responde.** Abre un chat más grande para seguir: texto en tiempo real con Markdown, LaTeX y código, todo copiable. Esc lo cierra.
- **Su estado se ve en la animación**, como las mascotas de ChatGPT/Codex:
  - trabajando;
  - necesita tu decisión;
  - listo sin leer;
  - bloqueado;
  - escuchando.
- **Orquestador.** Buddy es el único agente visible y pasa las tareas a especialistas por dentro. Los especialistas se crean y editan en Ajustes. PARLEY es el primero.
- **«Hey Buddy» con la mascota oculta.**
  - Palabra de activación local en Mac y Windows (openWakeWord, modelo propio).
  - En Mac, además: Siri y Atajos con «Oye Siri, pregúntale a Buddy…», vía App Intents.
- **Voz en las dos plataformas, sin enviar audio a la nube:**
  - Mac: `SpeechAnalyzer`, en el dispositivo.
  - Windows: whisper.cpp local, con un modelo pequeño que se descarga una vez.

### 1.2 El notch (Mac) / la barra superior (Windows)

- **Notificaciones sobrias, una a la vez.**
  - Claude, Codex o Gemini terminó en tal proyecto.
  - Pide permiso: Aceptar o Rechazar ahí mismo, como Claude Island / Vibe Notch.
  - Te queda un 5 % de uso.
  - Picks nuevos de PARLEY.
- **Música:** Spotify, Música y navegador.
- **«Suelta tus archivos aquí»:** dárselo a Buddy, o compartir por Gmail, Outlook o Teams (abre el redactor con el archivo; nunca envía solo).
- **El historial de chats.**
- **Uso de tus planes** de un vistazo.
- **Sin agentes en píldoras ni LED.**

### 1.3 Mensajitos útiles

- **Qué son.** Frases cortas sobre lo **interesante del día**: noticias de ingeniería y tecnología, deportes (resultados, lesiones, previas de tus ligas), lo que elijas en Ajustes.
- **Cómo se hacen.** Salen de búsqueda web con el modelo más barato: Haiku 4.5 o GPT-5.6 Luna, esfuerzo bajo.
- **Para no gastar:**
  - un resumen a primera hora (la base del día);
  - como mucho 2 o 3 actualizaciones al día;
  - cada una con un tope de búsquedas;
  - sin dato nuevo, no se dice nada.
- **Avisos de uso** («te queda 5 % de Claude»): los lee la app de tus límites, sin usar modelo.

### 1.4 Proveedores (tus suscripciones)

| Proveedor | Cómo entra | Para qué |
| --- | --- | --- |
| Claude (tu plan) | Claude Code CLI (`claude -p`, stream-json), como MIKA | Chat, archivos, código |
| ChatGPT / Codex (tu plan) | `codex app-server`, como MIKA | Chat, Office, imágenes, código |
| Gemini (tu AI Pro) | **Antigravity CLI** (`agy -p --output-format stream-json`). El Gemini CLI con cuenta de Google se cerró el 18-06-2026. | Chat, búsqueda, segunda opinión |

- **Interconector.** Un solo «proveedor» abstracto en el núcleo: Buddy y cada especialista pueden cambiar de LLM con un ajuste, y si un plan se queda sin uso, Buddy pasa a otro y te avisa.
- **Fuera:** Perplexity, Instagram y SofaScore.
- **«Jeff»:** pendiente de aclarar qué es (no se encontró una app o API con ese nombre).
- **Más agentes en las notificaciones.** Además de Claude Code, Codex y Antigravity, se pueden sumar los adaptadores de hooks de Cursor y Copilot CLI, como hace CoPet. A Codex también se le puede seguir por sus sesiones JSONL, como respaldo de los hooks, como hace vibe-pet.

### 1.5 PARLEY, especialista en deportes

- **Ahora se centra en estadísticas:** fútbol, tenis, NBA y NFL, con bases históricas de 2 o 3 temporadas (equipos, jugadores, entrenadores, lesiones y problemas), además de las cuotas.
- **Parlays** que combinan deportes.
- **Fuentes gratuitas.** Detalle en `docs/research/2026-10-02-investigacion.md`.

| Deporte | Fuentes |
| --- | --- |
| Fútbol | football-data.co.uk (resultados y cuotas de cierre desde los 90), StatsBomb Open Data (eventos), API-Football plan gratis (100/día) |
| Tenis | Datasets de Jeff Sackmann (`tennis_atp`, `tennis_wta`, desde los 60), TennisMyLife |
| NBA | `nba_api` (estadísticas oficiales de stats.nba.com), balldontlie plan gratis |
| NFL | nflverse (jugada a jugada desde 1999, plantillas, calendario) |
| Cuotas | OddsPapi (Betano), como MIKA |

- **Base local.** Todo se descarga **una vez** a una base SQLite y se actualiza con lo nuevo. PARLEY consulta la base y busca en la web solo bajas, alineaciones o noticias de hoy.
- **Lo que ya funciona en MIKA se mantiene:**
  - un análisis principal al día, como base;
  - revisiones solo si hay algo nuevo;
  - Telegram como base de picks;
  - la página HTML estilo Scout con filtros.

### 1.6 Capacidades de Buddy

- **Archivos.** Lee y edita en carpetas que autorizas (lista en Ajustes), nunca fuera.
- **Office.** Crea Word, Excel y PowerPoint (OfficeTools de MIKA).
- **Capturas.** Toma capturas, y lee imágenes y PDF.
- **Ejecutar.** Corre código o comandos **solo con tu permiso**.
- **Plugins y conectores** vía MCP (los de claude.ai y ChatGPT que MIKA ya usa).

## 2. Reglas de diseño

1. **Mac y Windows a la vez, con la misma estética.** Toda función nueva busca solución para las dos: si una plataforma tiene algo nativo mejor (SpeechAnalyzer, Siri), la otra tiene su equivalente funcional.
2. **Ahorrar tokens es un requisito**, no un extra:
   - base diaria;
   - nada corre sin algo nuevo;
   - tope de búsquedas por turno;
   - modelo barato para lo simple;
   - pocas capturas;
   - caché de APIs;
   - medidor de tokens por función.
3. **Reposo a ~0 % de CPU.** Animaciones solo cuando hay algo que mostrar; respetan «reducir movimiento».
4. **Seguridad:**
   - el contenido externo es dato, nunca instrucción;
   - permiso explícito para ejecutar, editar fuera de lo autorizado o enviar;
   - secretos en Llavero o Administrador de credenciales;
   - sin telemetría.
5. **Antivirus.** Se evitan ganchos de teclado globales, la inyección en procesos, descargar ejecutables, los binarios sin firma, los empaquetadores y los scripts ocultos. La firma (Apple Developer en Mac, Azure Artifact Signing en Windows) se configura al hacer el primer build distribuible; el paso a paso irá en `docs/phases/08-firma-y-endurecimiento.md`.

## 3. Arquitectura en una línea

Un núcleo en Rust compartido (proveedores, orquestador, enrutador, base SQLite, PARLEY, Telegram, notificaciones) con interfaces nativas encima: SwiftUI en Mac (vía UniFFI) y Tauri en Windows. Detalle en `docs/ARCHITECTURE.md`.

## 4. Fases

Cada fase tiene su plan en `docs/phases/` antes de escribir código.

| Fase | Entrega | Se acepta cuando |
| --- | --- | --- |
| 0. Esqueleto | Núcleo Rust, app Mac y app Windows que arrancan con el mismo «hola» del núcleo; CI con pruebas. | Las dos abren; `cargo test` pasa. |
| 1. Buddy (mascota + chat) | Personaje pixel art con sus estados, compositor compacto, chat que se expande, streaming con Markdown/LaTeX/código, Claude y Codex, orquestador. | En Mac y Windows: escribes, el primer texto sale en menos de 1 s y el chat se expande sin saltos. |
| 2. Notch / barra | Notificaciones de sesiones (hooks de MIKA) con aceptar/rechazar, uso de planes, música, «suelta tus archivos», historial. | Una notificación a la vez, se va sola, ~0 % de CPU en reposo. |
| 3. Proveedores y ahorro | Antigravity (Gemini), interconector y cambio automático, medidor de tokens, enrutador barato/bueno. | El mismo trabajo con ≥50 % menos tokens que MIKA. |
| 4. Capacidades | Carpetas autorizadas, Office, capturas, ejecutar con permiso, MCP. | Crea un PPT en una carpeta tuya y pide permiso antes de ejecutar. |
| 5. PARLEY y estadísticas | Base histórica (fútbol, tenis, NBA, NFL), análisis diario, parlays, página HTML. | Picks con estadística citada de la base, sin buscar lo que ya está. |
| 6. Mensajitos y compartir | Resumen del día (ingeniería, deportes) con modelo barato; Gmail/Outlook/Teams. | Mensajes solo con dato nuevo; compartir en 2 clics. |
| 7. Voz y «Hey Buddy» | SpeechAnalyzer (Mac), whisper.cpp (Windows), palabra de activación local, Siri/Atajos en Mac. | Dices «Hey Buddy» con la mascota oculta y aparece escuchando. |
| 8. Firma y endurecimiento | Firma y notarización, pruebas de antivirus, actualizaciones, pantalla de diagnóstico. | VirusTotal y Defender limpios; Gatekeeper sin avisos. |

## 5. Pendiente de decidir

- Qué es «Jeff».
- Si las cuotas siguen (OddsPapi) además de las estadísticas.
