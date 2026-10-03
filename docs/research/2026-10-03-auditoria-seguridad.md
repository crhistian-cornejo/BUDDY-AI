# Auditoría de seguridad: núcleo, Mac y Windows

2026-10-03 · Solo lectura: no se cambió, compiló ni ejecutó nada. Tres revisiones independientes (núcleo Rust con
`hook/` y `telegram/`, app de Mac, app de Windows), contrastadas con las reglas de `CLAUDE.md`.

Cómo leer la columna «Estado»:
- **Leído**: el camino se releyó en el código después del informe y coincide.
- **Informe**: lo afirma la revisión; no se releyó.
- **Supuesto**: depende de algo que no se puede ver desde el repositorio (cómo se comporta Codex, agy, macOS o
  WebView2).

Ningún hallazgo se probó con un ataque real ni en una máquina con Windows. Los números de línea pueden moverse: había
cambios sin confirmar en varios de estos archivos mientras se revisaba.

## Corregido el 2026-10-03

Con pruebas que fallaban antes del cambio y pasan después (`cargo test --workspace`, la batería de Mac y la de
Windows). Nada de esto se vio funcionando en la app en marcha ni en una máquina con Windows.

| # | Qué cambió | Qué queda |
| --- | --- | --- |
| A1 | El núcleo pone el comando antes que la descripción del agente. En Mac y Windows la caja del comando se desplaza en vez de cortarse, y «Permitir» y «Permitir siempre» quedan apagados hasta llegar al final. | Probar el desplazamiento dentro del notch real. |
| M1 | «Permitir siempre» ya no guarda ni obedece reglas de solo programa (`git`, `ls`); reconoce intérpretes, envoltorios y herramientas de Windows sin importar mayúsculas, extensión o versión; rechaza paréntesis (`=(…)`, calificadores de zsh); no se aplica a herramientas MCP. Las reglas de solo programa ya guardadas dejan de valer. | La regla exacta sigue viéndose solo al pasar el ratón. |
| A2 | Lo que escriben terceros en Telegram llega a PARLEY con cada línea marcada «│ », así que no puede imitar las marcas de Buddy. Los turnos de PARLEY solo leen los adjuntos de su propio chat. | El modelo aún puede dejarse convencer por un mensaje, y el turno sigue teniendo web. Falta M3 (memoria y traspasos). |
| A3 | En Mac solo piden su icono los sitios que el núcleo informó (los que el agente buscó o abrió), nunca los enlaces escritos en la respuesta. Windows no tiene esta función. | — |
| A4 | Donde Codex no tiene perfil de permisos (Windows), su búsqueda web usa el índice en caché de OpenAI en vez de páginas en vivo. | En Windows los resultados de Codex pueden ser menos recientes. El aislamiento antiguo sigue leyendo todo el disco: falta portar el perfil de permisos. Sin probar en Windows. |

## Graves

| # | Qué pasa | Dónde | Estado |
| --- | --- | --- | --- |
| A1 | **La tarjeta de permiso enseña 6 líneas y «Permitir» sigue activo** con comandos de hasta 16 KB. La descripción que escribe el agente va antes que el comando y puede ocupar las 6 líneas. Un agente manipulado muestra `git status` y esconde otra orden debajo. | `apps/macos/Sources/Notch/NotchView.swift:582` · `apps/windows/src/bar/bar.css:114` · `core/src/sessions/format.rs:57` | Leído |
| A2 | **El texto de los grupos de Telegram llega a un turno de PARLEY con herramientas web**, entre marcas que un mensaje puede imitar («[Fin de datos consultados…]», «[Petición original]»). El turno «restringido» solo quita comandos, edición y pantalla. El servidor de documentos puede leer los adjuntos de todos los chats, no solo los de ese. | `core/src/telegram_account.rs:235` · `core/src/parley.rs:37,66` · `core/src/chat.rs:835,1122,154` | Leído |
| A3 | **Un enlace en una respuesta provoca una conexión sin clic**: se pide el icono del sitio al dibujar la respuesta. Un contenido malicioso puede hacer que el modelo escriba un enlace con datos en el nombre del sitio. Activado por defecto. | `apps/macos/Sources/Chat/Markdown/SourceIcons.swift:57` · `ChatMarkdownView.swift:68` · gemelo de Windows `icons.rs` sin revisar | Leído (Mac) |
| A4 | **Codex en Windows usa el aislamiento antiguo**, que según el propio comentario del código deja leer todo el disco, con búsqueda web activa. Contradice «solo carpetas autorizadas». | `core/src/providers/codex.rs:181-192` | Leído (configuración) · Supuesto (comportamiento) |

## Medios

| # | Qué pasa | Dónde | Estado |
| --- | --- | --- | --- |
| M1 | **«Permitir siempre» guarda reglas más amplias de lo que se ve.** `git -C x status` guarda `git`, que luego cubre `git -c alias…` (ejecuta otros programas). La lista de programas prohibidos es exacta y distingue mayúsculas: `python.exe`, `powershell.exe`, `cmd`, `env` pasan. No bloquea `=(…)` ni los calificadores de zsh. No mira qué herramienta pide el comando. La regla solo se ve al pasar el ratón. | `core/src/sessions/always.rs:10-77` · `sessions/mod.rs:570` | Leído |
| M2 | **Gemini solo está confinado por archivos de reglas** que el propio código reconoce poco fiables (ya ignoró una regla de búsqueda web). | `core/src/providers/gemini.rs:180-245` · `chat.rs:855` | Supuesto |
| M3 | **El contenido externo puede escribir en la memoria y provocar traspasos.** Cualquier línea `[[recuerda]]` de una respuesta se guarda y entra en las instrucciones de todos los agentes; `[[pasar:…]]` se obedece aunque el turno ya haya leído la web. | `core/src/memory.rs:49-68` · `chat.rs:500,657` | Leído (memoria) · Informe (traspaso) |
| M4 | **Niko: la revisión programada lee correos en el mismo turno que puede escribir en Notion.** La separación en dos turnos solo existe en el camino IMAP, y aun ahí pasan `comercio` y `concepto` tal cual. | `core/src/niko.rs:561-576,1064-1101` | Informe |
| M5 | **Los comandos de los agentes heredarían los permisos de macOS de Buddy** (Accesibilidad, grabación de pantalla, micrófono). | `core/src/providers/process.rs:52` · `NotchSystemMonitor.swift:113` | Supuesto |
| M6 | **Los enlaces «de tu espacio» son clicables con cualquier etiqueta**, y GitHub, Google Forms, Drive y Notion alojan contenido de cualquiera. No se muestra la dirección. | `MarkdownSources.swift:11` · `apps/windows/src/chat/markdown.ts:308` | Informe |
| M7 | **La tarjeta de documento abre cualquier tipo de archivo con un clic y oculta la extensión.** | `apps/macos/Sources/Chat/ChatViews.swift:704` · `apps/windows/src-tauri/src/lib.rs:785` | Informe |

## Bajos

- **La comprobación de «carpeta demasiado amplia» no funciona en Windows** (`canonicalize` devuelve `\\?\C:\…` y
  nada coincide): acepta `C:\` o todo el perfil. En Mac acepta `~/.ssh`, `~/Library` y la carpeta de datos de
  Buddy. `core/src/folders.rs:54-77`. Leído.
- **Los comandos de Tauri confían del todo en la ventana**: `chat_id` sin validar se usa en una ruta,
  `open_shortcut` abre cualquier ruta, y las seis ventanas tienen el mismo permiso. Hoy no hay forma de inyectar
  código en la ventana; es defensa en profundidad. `apps/windows/src-tauri/src/lib.rs:767,951`. Leído (`open_shortcut`).
- **El núcleo confía en quien lo llama**: `set_setting` escribe cualquier clave, incluidas las de carpetas y
  permisos. `core/src/lib.rs:303`. Informe.
- **La tubería de Windows falla abierta** si no puede crear el descriptor de solo dueño.
  `core/src/sessions/server.rs:500`. Informe.
- **Sin pausa de seguridad entre tarjetas de permiso en Windows**: un doble clic puede aprobar dos seguidas.
  `apps/windows/src/bar/main.ts:366`. Informe.
- **Soltar un archivo sobre la mascota en Windows** probablemente navega su ventana. Supuesto.
- **Permisos revocados siguen vivos** en chats de Codex ya abiertos. `core/src/providers/codex.rs:43`. Informe.
- **Las instrucciones del agente van como argumento** (visibles para otros procesos) y el secreto de la puerta va
  en el entorno que heredan los comandos. `core/src/providers/claude.rs:170,388`. Informe.
- **El `state` de Spotify no es aleatorio** (reloj más pid). `core/src/spotify.rs:113`. Informe.
- **Texto de terceros enviado como mensaje propio con un clic** («No lo reconozco», botones de tarjetas). Informe.
- **Texto en claro**: los recortes del portapapeles van a la tabla de ajustes; el registro guarda asunto y
  remitente de correos. `core/src/notch.rs:79` · `niko.rs:676`. Informe.
- **El tiempo en el notch envía la IP a `ipwho.is` y las coordenadas a `open-meteo.com`** sin interruptor.
  `NotchWeather.swift:18`. Informe.
- **Contra la regla de antivirus**: un interceptor de eventos del sistema (solo teclas multimedia) y marcos
  privados en Mac. `NotchSystemMonitor.swift:141` · `NotchLock.swift:162`. Informe.
- **Firma**: Debug sin firma y Release ad hoc en Mac; sin firma ni `buddy-hook.exe` en el paquete de Windows
  (fase 8).
- **Dependencias**: `grammers-client 0.10` (previo a 1.0) y `glass_pumpkin` fijado a una versión candidata. No se
  pasó `cargo audit`.

## Lo que está bien

- Secretos solo en Llavero / Credential Manager; ninguno en SQLite, eventos, registros ni mensajes de error.
- SQL siempre con parámetros.
- Windows: ninguna forma de que un texto externo se convierta en código en la ventana (sin `innerHTML`, CSP
  estricta, nada desde CDN). Mac: sin vistas web; las imágenes de las respuestas no se cargan.
- Socket y tubería de sesiones solo del dueño, con comprobación de quién está al otro lado y límites de tamaño.
- La puerta de permisos falla cerrada; una petición recortada no se puede permitir; sin respuesta es «no».
- Capturas de pantalla solo tras la tarjeta «Ver tu pantalla». Dictado en el equipo.
- Bot de Telegram: emparejado, un solo chat, imágenes con límites. Lector de cuenta: solo lectura.
- IMAP con TLS verificado y solo lectura. Portapapeles solo al pulsar el botón.
- Sin atajos globales ni lectura de teclado; sin telemetría.

## No cubierto

- Ninguna prueba en ejecución; nada en una máquina con Windows.
- `hook/src/office/*` (solo límites), partes de `niko.rs`, `cards.rs`, `usage.rs`, `router.rs`, `telegram/` tras
  la línea 470, el gemelo `icons.rs` de Windows, las pruebas de Mac.
- Avisos conocidos de dependencias (`cargo audit`, `npm audit`).

## iPhone

No hay código que auditar todavía. El modelo de amenazas está en `docs/phases/10-iphone.md` y la etapa 4 audita lo
construido. De esta auditoría salen cuatro condiciones para esa fase:

1. **A1 y M1 se corrigen antes de aprobar permisos desde el teléfono.** La tarjeta del iPhone debe mostrar el
   comando entero, con desplazamiento, antes de dejar pulsar «Permitir».
2. **`chat_id` se valida en el núcleo** antes de aceptar `send_message` desde fuera.
3. **`set_setting` nunca entra en la lista de llamadas remotas**, y las claves reservadas se rechazan en el núcleo.
4. **A3:** la app de iPhone no pide iconos de sitios por enlaces del texto.

## Orden sugerido de corrección

1. A1 y M1: lo que se ve en la tarjeta es lo que se aprueba.
2. A3: iconos solo de fuentes que el núcleo confirmó, o apagado por defecto.
3. A2 y M3: el contenido externo se resume en un turno sin herramientas y no escribe memoria.
4. A4: Codex en Windows sin web, o fuera del enrutado, hasta tener perfil de permisos.
5. El resto, por lotes.
