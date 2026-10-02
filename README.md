# Buddy

Asistente personal de escritorio para **macOS** (Swift) y **Windows** (Tauri + Rust), con la misma estética en las dos.

- **Buddy, la mascota.** Un personaje en pixel art, dibujado en código, que vive en una esquina de la pantalla.
  - Le escribes o le hablas, y él orquesta: decide si responde él o pasa la tarea a un especialista (PARLEY para deportes, documentos, código…).
  - Funciona con tus suscripciones de Claude, ChatGPT/Codex y Gemini (vía Antigravity).
- **El notch** (Mac) o la barra superior (Windows):
  - Notificaciones sobrias: Claude, Codex o Gemini terminó o pide permiso; te queda poco uso.
  - Música.
  - "Suelta tus archivos aquí", para compartir o dárselos a Buddy.
  - El historial de chats.

Buddy nace de [MIKA](../Mac_Windows/MIKA): reutiliza sus piezas probadas y deja atrás lo que sobraba.

## Dónde está cada cosa

| Carpeta | Qué hay |
| --- | --- |
| `docs/PLAN-MAESTRO.md` | El plan: qué es Buddy, decisiones y fases. **Empieza aquí.** |
| `docs/ARCHITECTURE.md` | Cómo encaja todo: núcleo Rust compartido e interfaces nativas. |
| `docs/REUSE-FROM-MIKA.md` | Qué se toma de MIKA, de dónde y qué cambia. |
| `docs/design/` | La mascota en pixel art y el notch. |
| `docs/research/` | Lo investigado, con fuentes. |
| `docs/phases/` | El plan de cada fase. |
| `core/` | Núcleo en Rust (proveedores, orquestador, datos). Fase 0. |
| `apps/macos/` | App de Mac (SwiftUI + núcleo). Fase 0. |
| `apps/windows/` | App de Windows (Tauri + núcleo). Fase 0. |
| `assets/sounds/` | Los sonidos de MIKA (generados por código). |
| `reuse/mika/` | Copia de referencia de MIKA (revisión en `reuse/mika/REVISION`), de donde se portan componentes. **No se compila.** |

## Estado

Fase 0 (esqueleto) hecha: núcleo Rust con pruebas, app de Mac (SwiftUI + UniFFI) y app de Windows (Tauri 2) que pintan a Buddy («Mochi») desde el mismo núcleo, y CI. Siguiente: fase 1 (mascota + chat).

## Compilar

```bash
cargo test --workspace                     # núcleo + app de Windows (necesita apps/windows/dist: npm run build)
cd apps/macos && xcodegen && xcodebuild -project Buddy.xcodeproj -scheme Buddy build
cd apps/windows && npm ci && npm run build && npx tauri dev
```
