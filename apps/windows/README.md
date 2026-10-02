# apps/windows

App de Windows en Tauri 2 sobre `core` (Rust directo) con interfaz TypeScript/Canvas: mascota, barra superior, chat, voz (whisper.cpp).

```bash
npm ci
npm test            # pruebas de la interfaz (vitest)
npx tauri dev       # también arranca en un Mac, para comparar con la app nativa
```

Los íconos salen del personaje del núcleo: `python3 scripts/make-icons.py`.
