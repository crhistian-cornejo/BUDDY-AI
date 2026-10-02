# Buddy: arquitectura

## Visión general

```
                 ┌──────────────────────────── buddy-core (Rust) ────────────────────────────┐
                 │  providers/   Claude (CLI stream-json) · Codex (app-server) · Antigravity    │
                 │  orchestrator/ Buddy + especialistas (hand-off), enrutador barato/bueno      │
                 │  store/       SQLite: chats, memoria, picks, estadísticas, caché, uso        │
                 │  usage/       límites de cada plan, medidor de tokens por función            │
                 │  sessions/    hooks de Claude Code / Codex (servidor de sockets)             │
                 │  parley/      análisis diario, base, Telegram, estadísticas, página HTML     │
                 │  briefing/    mensajitos del día (modelo barato + búsqueda con tope)         │
                 │  voice/       palabra de activación (openWakeWord ONNX) · whisper (Windows)   │
                 │  events       un flujo de eventos tipados → las interfaces solo dibujan      │
                 └───────────────▲──────────────────────────────────────────▲────────────────┘
                                 │ UniFFI (Swift bindings)                  │ Rust directo
                 ┌───────────────┴──────────────┐              ┌────────────┴───────────────┐
                 │ apps/macos (SwiftUI/AppKit)  │              │ apps/windows (Tauri 2)      │
                 │ notch · mascota · chat       │              │ barra · mascota · chat      │
                 │ SpeechAnalyzer · App Intents │              │ (TypeScript + Canvas)       │
                 └──────────────────────────────┘              └─────────────────────────────┘
```

## Principios

1. **La lógica vive una sola vez, en `core/`.** Las apps no deciden nada de proveedores, datos ni reglas: llaman al núcleo y dibujan sus eventos. Así Mac y Windows no se separan, como pasaba en MIKA.
2. **Eventos, no sondeos.** El núcleo emite eventos tipados (`TextDelta`, `ToolStatus`, `SessionFinished`, `NeedsApproval`, `UsageLow`, `PickNew`, `BriefingReady`…). Nada se refresca con temporizadores mientras no hay actividad.
3. **Lo nativo, nativo.**
   - Ventanas transparentes del notch y la mascota, la voz en Mac, Siri y los permisos van en la app de cada plataforma.
   - Todo lo demás (estado, red, modelos) está en el núcleo.
4. **Estética única.**
   - Las dos apps comparten tokens de diseño en `assets/design-tokens.json` (colores, radios, tipografía, tiempos de animación), que cada plataforma lee.
   - Comparten también el mismo dibujo del personaje: la definición de los sprites vive en el núcleo y cada app la pinta.
5. **Pruebas donde está la lógica.** `cargo test` en el núcleo. En las apps, pruebas solo de lo propio de su interfaz.

## Proveedores e interconector

```rust
trait Provider {
    fn id(&self) -> ProviderId;                 // claude | codex | antigravity
    fn status(&self) -> ProviderStatus;          // instalado, conectado, uso restante
    fn run(&self, turn: Turn) -> EventStream;    // streaming; cancelable
}
```

- **Las tres implementaciones salen de MIKA:**
  - `ClaudeTurn` y `ClaudeStreamParser` (Swift) → Rust, con `services/claude.rs` de Windows como base;
  - `CodexClient` y `codex_server.rs`;
  - la nueva de Antigravity CLI.
- **El enrutador** elige el modelo según la tarea: barato para clasificar, resumir y los mensajitos; bueno para el chat y el trabajo. Si un plan está casi agotado, cambia de proveedor y avisa.
- **Respaldo por falta de créditos**, el `CreditFallback` de MIKA, generalizado.

## Datos

- **Base:** SQLite en la carpeta de datos de la app (`~/Library/Application Support/Buddy`, `%LOCALAPPDATA%\Buddy`).
- **Tablas:**
  - `chats`, `messages`, `memory`;
  - `picks`, `pick_results`;
  - `stats_*` (por deporte), `api_cache`;
  - `usage_events` (tokens por función).
- **Archivos grandes** (capturas de Telegram, adjuntos, páginas HTML) van en carpetas al lado, nunca en la base.

## Seguridad

- **Secretos** en el Llavero o el Administrador de credenciales; el núcleo pide la clave a la app y nunca la guarda.
- **Carpetas autorizadas.** Solo se lee o escribe dentro de la lista de carpetas del usuario; todo lo demás se rechaza en el núcleo.
- **Ejecutar comandos** pasa por la puerta de aprobación (el `AgentGate` de MIKA): burbuja con Aceptar o Rechazar.
- **Contenido externo** (web, archivos, Telegram) se marca como dato en todos los prompts.
