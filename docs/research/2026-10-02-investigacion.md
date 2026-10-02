# Investigación (2026-10-02)

Fuentes consultadas para el plan de Buddy. Se revisan cuando una fase dependa de ellas, porque las APIs cambian.

## Mascotas de escritorio para agentes (lo más cercano a Buddy)

| Proyecto | Qué es | Qué tomamos |
| --- | --- | --- |
| Mascotas de ChatGPT/Codex ([docs](https://learn.chatgpt.com/docs/pets)) | Pixel art animado que flota sobre las ventanas desde mayo de 2026. 4 estados con prioridad (necesita decisión > bloqueado > listo > trabajando). Al pasar el mouse muestra lápiz, micrófono y campana. Opción+Espacio / Win+Alt+P. Respeta «reducir movimiento». Las mascotas propias son spritesheets de 1536 × 1872. | Modelo de estados y prioridades, controles al pasar el mouse, atajo, «reducir movimiento». Opción de **importar** mascotas en ese formato. |
| [CoPet](https://github.com/ChanceYu/CoPet) (MIT) | Mascota que reacciona a los agentes. **Rust + Tauri + React**, macOS y Windows. Hooks para Claude Code (`~/.claude/settings.json`), Codex (`~/.codex/hooks.json`), Cursor, Copilot CLI. Personajes con `pet.json` + `spritesheet.webp`; paquetes de 11 sonidos; datos en `~/.copet`, sin telemetría. | Mismo stack que la app de Windows. Adaptadores de hooks para más agentes, formato `pet.json`, paquetes de sonido. Al ser MIT se puede reutilizar código citándolo. |
| [vibe-pet](https://github.com/Seeed-Solution/vibe-pet) (MIT) | Electron; mascotas por agente (Cursor, Codex, Windsurf, Claude, Gemini, Copilot). Personajes de la biblioteca [Petdex](https://github.com/topics/codex-pets). Codex por **sesiones JSONL** además de hooks; sincroniza con pantallas físicas (ESP32) por BLE. | Leer las sesiones JSONL de Codex como respaldo de los hooks; importar personajes de Petdex (revisar la licencia de cada uno). |
| Clawd, la mascota de Claude Code ([Stark Insider](https://www.starkinsider.com/2025/10/clawd-ai-retro-mascot-command-line.html)) | Sprite de 8 bits en la terminal. Proyectos como [ClawdMoji](https://kompozy.io/ai-tools/clawdmoji) lo generan **desde una sola cuadrícula de píxeles definida a mano**; [claude-pet](https://github.com/xtrimsystems/claude-pet) lo pone como mascota que refleja la sesión. | El **estilo**: pixel art pequeño, definido en código como cuadrícula. Clawd es de Anthropic: Buddy se inspira en el estilo pero usa personajes propios. |

## Apps de notch

| App | Plataforma | Qué hace bien |
| --- | --- | --- |
| [NotchNook, Alcove, Boring Notch](https://alternativeto.net/software/notchnook) | Mac | Música, bandeja de archivos, notificaciones, HUD |
| [Claude Island / Vibe Notch](https://alternativeto.net/software/claude-island/about) | Mac | Sesiones de Claude Code/Codex/Gemini con aprobar/denegar y saltar a la terminal |
| [Venu](https://peerlist.io/imnakul/project/venu-dynamic-notch-for-windows) | Windows | Media, notificaciones, foco, **uso de herramientas de IA de código** |
| [Notchify](https://www.windowscentral.com/software-apps/notchify-brings-macos-style-dynamic-island-flair-to-windows-11), [DynamicWin](https://github.com/FlorianButz/DynamicWin), [Bubble](https://www.producthunt.com/p/bubble-5/bubble-b9757286-26aa-43a5-b575-0326828dfa6b) | Windows | Isla con portapapeles; asistente de voz con isla |

## Proveedores

| Tema | Hallazgo |
| --- | --- |
| Gemini con tu AI Pro | El acceso con cuenta de Google al Gemini CLI terminó el 18-06-2026 ([geminicli.com](https://geminicli.com/docs/resources/quota-and-pricing/)); **Antigravity CLI** lo reemplaza sin costo extra en cada plan ([letsdatascience](https://letsdatascience.com/news/google-replaces-gemini-cli-with-antigravity-cli-574b2056)). |
| Antigravity sin interfaz | `agy -p` con `--output-format stream-json` (NDJSON), `--json-schema` y `--yes` ([docs](https://antigravity.google/docs/cli/headless)). |
| «Jeff» | No se encontró una app, API ni agente con ese nombre ([búsqueda](https://growwstacks.com/blog/ai-agent-desktop-app-multi-llm-browser-control)). Pendiente de aclarar. |

## Voz y «Hey Buddy»

| Tema | Hallazgo | Decisión |
| --- | --- | --- |
| Mac | `SpeechAnalyzer` (macOS 26): en el dispositivo, sin red, mejor que Whisper en pruebas ([WWDC25](https://developer-mdn.apple.com/videos/play/wwdc2025/277/)) | Usar en Mac |
| Windows | `Windows.Media.SpeechRecognition` (modelo antiguo) o **whisper.cpp** local, sin dependencias, modelos desde 75 MB ([guía](https://voxbooster.com/blog/whisper-transcription-windows)) | whisper.cpp vía `whisper-rs` en el núcleo |
| Palabra de activación | **openWakeWord**: código Apache-2.0, modelos ONNX, se entrena una palabra propia en ~1 h; sus modelos preentrenados son CC BY-NC-SA ([PyPI](https://pypi.org/project/openwakeword)). Porcupine es multiplataforma pero su SDK de Rust dejó de mantenerse en 2025 ([docs](https://picovoice.ai/docs/api/porcupine-rust)). | openWakeWord con un modelo «Hey Buddy» propio, en el núcleo (ONNX Runtime) para las dos plataformas |
| Siri en Mac | App Intents / Atajos: «Oye Siri, pregúntale a Buddy…» | Extra solo de Mac |

## Estadísticas deportivas (gratis, históricas)

| Deporte | Fuente | Notas |
| --- | --- | --- |
| Fútbol | [football-data.co.uk](https://sportsapis.dev/free-sports-datasets) | Décadas de resultados de ligas europeas con cuotas de cierre (MIKA ya lo usa) |
| Fútbol | StatsBomb Open Data | Eventos de competiciones seleccionadas; revisar licencia |
| Fútbol | API-Football (gratis) | 100 solicitudes/día; temporada actual limitada |
| Tenis | Datasets de Jeff Sackmann (`tennis_atp`, `tennis_wta`) y TennisMyLife | CC BY-NC-SA: uso personal |
| NBA | `nba_api` (stats.nba.com) y balldontlie (gratis) | No oficial pero muy usado; balldontlie limita a 5 solicitudes/minuto |
| NFL | [nflverse](https://sportsapis.dev/free-sports-datasets) | Jugada a jugada desde 1999, plantillas y calendario |
| Varios | [SportsDataverse](https://github.com/sportsdataverse/sportsdataverse-py/wiki) | Paquetes para 18 ligas |

Ojo: descargar gratis no significa permiso de uso comercial ([unidata](https://unidata.pro/blog/best-free-sports-datasets-ml)). Buddy es de uso personal.

## Firma y antivirus (para la fase 8)

| Plataforma | Qué hace falta |
| --- | --- |
| Windows | Firmar con **Azure Artifact Signing** (~$10/mes, sin token físico, integra con GitHub Actions). La reputación de SmartScreen se gana con semanas de instalaciones limpias ([Tauri](https://v2.tauri.app/distribute/sign/windows/), [Microsoft](https://star-hk2.eastasia.cloudapp.azure.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation)) |
| Mac | Developer ID (el dueño ya lo tiene) + notarización |
