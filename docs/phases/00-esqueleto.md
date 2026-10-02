# Fase 0: el esqueleto

**Objetivo:** las dos apps arrancan sobre el mismo núcleo y la base para todo lo demás queda lista.

## Entregables

1. **`core/`**
   - Crate Rust `buddy-core` con su `Cargo.toml` y un workspace en la raíz.
   - Módulos vacíos con su responsabilidad documentada: `providers`, `orchestrator`, `router`, `store`, `sessions`, `usage`, `parley`, `briefing`, `voice`, `events`, `log`, `pixel`.
   - `events.rs`: el tipo `Event` y un canal por suscriptor.
   - `pixel.rs`: el formato de personajes de `docs/design/MASCOTA-PIXEL.md`, con su validación y el primer personaje `buddy-base` (reposo, 2 fotogramas).
   - `store.rs`: abrir o crear la base SQLite en la carpeta de datos de la app, con migraciones versionadas.
   - Pruebas con `cargo test`: validación de personajes, migraciones, eventos.
2. **`apps/macos/`**
   - `project.yml` (XcodeGen), app `Buddy` con bundle id `io.github.crhistian-cornejo.buddy` y LSUIElement (sin Dock).
   - El núcleo enlazado vía UniFFI.
   - Una ventana flotante transparente que pinta `buddy-base` desde el núcleo, con escala entera y sin suavizado.
   - Arrastrable; la posición se guarda.
3. **`apps/windows/`**
   - App Tauri 2 `Buddy` con el núcleo como dependencia de ruta.
   - La misma ventana flotante, pintando el mismo personaje en Canvas 2D.
4. **`assets/design-tokens.json`:** colores, radios, tipografía y tiempos que leen las dos apps.
5. **CI (GitHub Actions):** `cargo test` en Mac y Windows; build de la app de Mac y de la de Windows.

## Se acepta cuando

- [ ] `cargo test` pasa en el núcleo.
- [ ] La app de Mac abre y muestra a Buddy pixel art flotando, arrastrable, a ~0 % de CPU en reposo.
- [ ] La app de Windows abre y muestra a Buddy idéntico.
- [ ] Un mismo cambio en `core/src/pixel` cambia a Buddy en las dos.

## No entra

Chat, proveedores, notch: son fases 1 y 2.
