# Qué se reutiliza de MIKA

La copia de referencia está en `reuse/mika/`; su revisión está en `reuse/mika/REVISION`. Nada de esa carpeta se compila: cada pieza se porta a su lugar en Buddy, con sus pruebas, en la fase que la necesita.

**Leyenda**

| Marca | Significa |
| --- | --- |
| **Portar a Rust** | Pasa al núcleo compartido (`core/`). La versión de Windows ya está en Rust y sirve de base. |
| **Mac** / **Windows** | Se queda como interfaz nativa de esa plataforma. |
| **Reescribir** | La idea sirve, el código no. |
| **No** | Se deja. |

## Proveedores y chat (fase 1)

| Pieza en MIKA | Mac | Windows | En Buddy |
| --- | --- | --- | --- |
| Turnos de Claude con streaming | `Providers/ClaudeTurn.swift`, `ClaudeStreamParser.swift` | `services/claude.rs`, `chat_frames.rs` | **Portar a Rust** → `core/providers/claude` |
| Codex app-server | `Providers/CodexClient.swift`, `CodexConnectors.swift` | `services/codex_server.rs` | **Portar a Rust** → `core/providers/codex` |
| Procesos y entorno limpio | `Providers/ProviderProcess.swift` | `services/subscription.rs` | **Portar a Rust** |
| Eventos, estado y modelos | `ProviderEvent.swift`, `ProviderStatus.swift`, `ModelCatalog.swift`, `CreditFallback.swift`, `MessageError.swift`, `ToolStatus.swift` | `services/subscription.rs` | **Portar a Rust** → `core/providers`, `core/router` |
| Pase entre agentes | `Providers/Handoff.swift` | `services/handoff.rs` | **Portar a Rust** → `core/orchestrator` |
| Definición de agentes y especialistas | `AgentDefinition.swift`, `AgentCapabilities.swift`, `AgentPrompt.swift`, `AgentSkills.swift` | `named_agents.rs`, `agent_skills.rs` | **Portar a Rust**; sin «look» de personaje |
| Memoria y archivo de chats | `AgentMemory.swift`, `ChatArchive.swift`, `ChatModel.swift` | `chat_store.rs` | **Portar a Rust** → `core/store` (SQLite) |
| Adjuntos por capacidad | `Providers/Attachments.swift` | `services/attachments.rs` | **Portar a Rust** |
| Markdown, LaTeX, código | `Providers/Markdown/*`, `Island/ChatMarkdownView.swift` | `core/markdown.ts`, `tex.ts`, `highlight.ts`, `views/answer.ts` | **Mac** (vista) y **Windows** (vista); los dos parsers se comparan en pruebas |
| Fuentes como íconos | `SourceIcons.swift`, `SourceLogos.swift` | `views/service-marks.ts` | **Mac** / **Windows** |

## Herramientas, permisos y conectores (fase 4)

| Pieza | Mac | Windows | En Buddy |
| --- | --- | --- | --- |
| Puerta de aprobación | `AgentGate.swift`, `Island/IslandGatePresenter.swift` | `agent_gate.rs`, `core/approval.ts` | **Portar a Rust** (regla) + vista en cada app |
| Herramientas de archivos | `AgentTools.swift`, `FileLimits.swift` | `agent_tools.rs`, `files.rs` | **Portar a Rust**, con la lista de carpetas autorizadas |
| Word, Excel, PowerPoint | `OfficeFiles.swift`, `OfficeTools.swift`, `OfficeAnnounce.swift` | — | **Portar a Rust** |
| Conectores MCP (claude.ai, ChatGPT) | `Connectors.swift`, `CodexConnectors.swift` | — | **Portar a Rust** |
| Contexto de la ventana activa | `Island/WindowContextCapture.swift` | — | **Mac**, más un equivalente en **Windows** |

## Notch y sesiones (fase 2)

| Pieza | Mac | Windows | En Buddy |
| --- | --- | --- | --- |
| Servidor de hooks de Claude Code / Codex | `Agents/ClaudeCode/HookServer.swift`, `ClaudeHooksInstaller.swift`, `Codex/CodexHooksInstaller.swift`, `PermissionRequestFormatter.swift` | `agents/claude_code`, `agents/codex`, `hook/` (el ejecutable de relevo) | **Portar a Rust** → `core/sessions`; el relevo se mantiene |
| Ventana del notch y su máquina de estados | `IslandWindowController.swift`, `IslandStateMachine.swift`, `IslandRootView.swift` (forma, orejas) | `platform/island.rs`, `island/fsm.ts`, `island/island.ts` | **Mac** / **Windows**, simplificado: solo notificación, música, archivos e historial |
| Barra de música | `Island/MediaStripView.swift`, `Services/MediaControl.swift` | `services/media.rs`, `views/media-strip.ts` | **Mac** / **Windows** tal cual (ya con transición y espacios) |
| «Suelta tus archivos aquí» | `Island/FileDropView.swift`, `Character/UploadCanvasView.swift` | `upload/*`, `views/upload.ts` | **Reescribir** la animación con el personaje pixel; la lógica de soltar se mantiene |
| Historial | `Island/HistoryView.swift` | `views/history.ts`, `core/history.ts` | **Mac** / **Windows** |
| Uso de los planes | `Services/Limits.swift`, `UsageRefresher.swift`, `Island/UsageLine.swift`, `UsageBubble.swift` | `services/limits.rs`, `island/usage-line.ts` | **Portar a Rust** (lectura) + vista |
| Notificación del sistema | `Services/Notifier.swift` | — | **Mac**, más un equivalente con notificaciones de **Windows** |
| Burbuja bajo el notch con botones | `Island/Tips.swift` (`notice`) | `pet/notice.ts` | **Reescribir** con la estética nueva |
| Tooltips | `Island/Tooltip.swift`, `TooltipLogic.swift` | `core/tooltip*.ts` | **Mac** / **Windows** |
| Sonidos | `Services/SoundEngine.swift` | `core/sound.ts` | **Mac** / **Windows**; los WAV ya están en `assets/sounds/` |

## Mascota (fase 1)

| Pieza | Mac | Windows | En Buddy |
| --- | --- | --- | --- |
| Ventana flotante de la mascota, arrastrar, posición | `Island/PetController.swift`, `Satellite.swift` | `platform/pet.rs`, `pet/*` | **Mac** / **Windows**: la ventana sí; el dibujo no |
| Personaje (cuerpo redondo con ojos, sombrero, ropa) | `Character/*`, `MikaAvatarView.swift` | `character/*` | **No**: Buddy es pixel art nuevo (`docs/design/MASCOTA-PIXEL.md`), sin la herencia del personaje de Coucou |
| Menú radial | `RingLayout.swift` | `pet/layout.ts` | Estilo opcional «radial»; el definitivo es el simple |
| LED, ticker, píldoras de agentes | `LedBoardView.swift`, `LedFont.swift`, `TeamCardView.swift`, `TipDeck.swift` | `island/led.ts`, `team-rail.ts`, `views/ticker.ts` | **No** |

## Integraciones y PARLEY (fases 5 y 6)

| Pieza | Mac | Windows | En Buddy |
| --- | --- | --- | --- |
| Telegram (lector propio, helper) | `Integrations/Telegram/*` | `integrations/telegram.rs`, `telegram/` (helper) | **Portar a Rust**; el helper se mantiene firmado |
| PARLEY: reglas, revisiones, análisis diario, página | `Picks.swift`, `PicksService.swift`, `ParleyDigest.swift`, `ParleyDigestPlan.swift`, `StatModel.swift` | `picks.rs` | **Portar a Rust** → `core/parley`; la página HTML se genera en el núcleo |
| Cuotas y estadísticas | `Odds.swift`, `Sgo.swift`, `FreeStats.swift` | `odds.rs`, `odds_reader.rs`, `sgo.rs`, `stats.rs` | **Portar a Rust**; SGO solo si se mantiene; se suman las fuentes nuevas (NBA, NFL, tenis histórico) |
| Rutinas | `Routines.swift`, `RoutineService.swift` | — | **Portar a Rust** si se usan (para los mensajitos) |

## Ajustes y base de la app (fase 0)

| Pieza | Mac | Windows | En Buddy |
| --- | --- | --- | --- |
| Registro local sin secretos | `Services/MikaLog.swift` | `services/log.rs` | **Portar a Rust** (`core/log`) |
| Secretos en Llavero / Credential Manager | (Keychain en `ClaudeService`, `KeychainStore`) | `services/secrets.rs` | **Mac** / **Windows** detrás de una interfaz del núcleo |
| Ajustes | `Settings/*` | `settings/*`, `services/settings.rs` | **Reescribir** con la estética nueva; el modelo de ajustes pasa al núcleo |
| Iniciar con el sistema | `SMAppService` en `SettingsView.swift` | plugin `autostart` | **Mac** / **Windows** |
| Enlaces externos seguros | `ExternalLink.swift` | `platform/shell.rs` | **Portar a Rust** (validación) |
