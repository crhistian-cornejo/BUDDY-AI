# core (buddy-core, Rust)

El núcleo compartido por las dos apps: proveedores, orquestador, enrutador, base SQLite, sesiones (hooks), uso, PARLEY, Telegram, mensajitos, voz y el formato de los personajes pixel.

- `src/lib.rs`: `BuddyCore`, el objeto que abre cada app (datos, ajustes, personajes, eventos).
- `src/pixel.rs` + `characters/*.json`: personajes en cuadrícula, validados y rasterizados a ARGB.
- `src/store.rs`: SQLite con migraciones versionadas (`PRAGMA user_version`).
- `src/events.rs`: eventos tipados, un canal por suscriptor.
- Feature `ffi`: los enlaces Swift con UniFFI (`scripts/build-core-macos.sh`, `uniffi-bindgen/`).

```bash
cargo test -p buddy-core
```
