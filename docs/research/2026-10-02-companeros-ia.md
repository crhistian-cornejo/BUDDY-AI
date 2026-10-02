# Compañeros de IA de escritorio (2026-10-02)

Lo que el dueño pidió mirar: Muse de Meta, «Brockbox», «DOPS de OpenAI» y los «Claude mods». Son nombres dictados por voz; abajo está lo que se identificó. Las fuentes son de prensa y comunidad, sin documentación oficial abierta, así que se revisan antes de la fase que dependa de ellas.

## Qué es cada uno

| Nombre dictado | Lo más probable | Qué es | Fuentes |
| --- | --- | --- | --- |
| Muse (Meta) | **Meta Muse** | Agente personal (correo, viajes, navegar, formularios, pagos). Salió el 8-sep-2026 en EE. UU. y la app de Mac el 17 o 18-sep. Invocación rápida (⌥Espacio), dictado en cualquier app, adjuntar una ventana, tareas programadas, briefings, memoria «About Me». Trabaja en segundo plano y deja registro de lo hecho y lo planeado. Las acciones sensibles se aprueban, y un agente «Sentinel» controla las salidas a la red. | [ajc](https://www.ajc.com/news/2026/09/meta-launches-personal-ai-agent-muse-emphasizes-safety-and-privacy/), [iphoneincanada](https://www.iphoneincanada.ca/2026/09/18/meta-launches-muse-on-macos-to-take-on-apple-intelligence/), [remio](https://www.remio.ai/post/meta-muse-for-mac-can-act-across-your-desktop-but-permission-is-the-product) |
| «Brockbox» | **Grok Bot** (xAI con Cursor), sin confirmar | Agentes siempre activos, cada uno con su ordenador en la nube (beta desde el 11-ago-2026). Solo vuelven para pedir aprobación o al terminar. Un bot dirige a especialistas y aprende rutinas como skills. | [igeeksblog](https://www.igeeksblog.com/grok-bot-macos-ios-launch/), [letsdatascience](https://letsdatascience.com/news/grok-bot-launches-persistent-workplace-agents-d3705e3d) |
| «DOPS de OpenAI» | **Codex Pets** | Mascotas pixel art sobre las ventanas en Mac y Windows (2-may-2026). Tienen cuatro estados: Running, Needs input, Ready y Blocked. Al hacer clic se responde al agente; `/pet` y `/hatch`. Formato abierto: `~/.codex/pets/<id>/pet.json` + `spritesheet.webp` (1536 × 1872, 8 × 9 celdas de 192 × 208). | [letsdatascience](https://letsdatascience.com/news/openai-introduces-codex-pets-animated-coding-companions-f3125f76), [penchan](https://penchan.co/en/ai/coding/codex-pets/) |
| «Claude mods» | **Mods de Claude Code** | Plugins de «function hooks» en TS/JS que añaden paneles en vivo, franjas, status line y toasts, y que pueden bloquear o reescribir herramientas. La comunidad hizo «code-pet», una mascota que reacciona a cada herramienta, error o comando peligroso. | [claude-code-mods](https://github.com/OneWave-AI/claude-code-mods), [aitmpl](https://www.aitmpl.com/component/mods/productivity/aitmpl) |

## Qué toma Buddy

| Idea | En Buddy | Fase |
| --- | --- | --- |
| Estados con prioridad (Codex Pets) | Ya está en el diseño: necesita decisión > bloqueado > listo > trabajando, emitidos por el núcleo | 1 |
| Clic en la mascota para responder o aprobar | Burbuja con aprobar/denegar o un campo de respuesta, unida a los hooks | 1–2 |
| Reacciones de la mascota (code-pet) | Reacciona a cada herramienta, a los errores y a los comandos peligrosos; duerme sin actividad. Solo anima con eventos | 1 |
| Importar mascotas `.codex-pet` | Importador en el núcleo (`pet.json` + spritesheet 8 × 9) que asigna cada fila a un estado de Buddy | después de 1 |
| Invocación rápida (Muse) | Atajo del sistema (sin ganchos globales) que abre el compositor de la mascota | 1 |
| Adjuntar ventana | Con permiso: texto de accesibilidad y una sola captura | 4 |
| Briefings, «About Me», tareas programadas | Mensajitos del día y memoria editable en SQLite | 6 |
| Registro de lo hecho y lo planeado | Historial del notch con el plan y las acciones de cada agente | 2 |
| Aprobación de acciones sensibles (Sentinel) | Puerta de aprobación del núcleo: muestra qué cambia y pide un clic | 4 |
| Un bot que dirige especialistas (Grok Bot) | El orquestador reparte el trabajo entre Claude, Codex y Gemini y vuelve solo para aprobar o al terminar | 1, 3 |
| Mod de Claude Code como puente | Mod «buddy-bridge» que manda eventos al socket local de Buddy (alternativa a los hooks) | 2 |
| Medidor de gasto | Medidor de tokens por función en el notch | 3 |
