# Fase 1: Buddy, mascota y chat

**Objetivo:** hablar con Buddy desde la mascota, en Mac y Windows, con Claude y Codex.

## Entregables

1. **Personajes pixel** en el núcleo: `buddy-base`, `claude`, `codex`, con los estados de `docs/design/MASCOTA-PIXEL.md` (reposo, pensando, trabajando, necesita decisión, bloqueado, listo, saludo, arrastrado) y la transición de traje.
2. **Proveedores en el núcleo**, portados de MIKA según `docs/REUSE-FROM-MIKA.md`:
   - Claude: CLI con stream-json.
   - Codex: app-server.
   - Comunes: estado de conexión, respaldo por falta de créditos y streaming de eventos.
3. **Orquestador.**
   - Buddy responde o pasa la tarea a un especialista (el hand-off de MIKA).
   - Los especialistas se definen en archivos como `agent.md` y se ven en Ajustes.
4. **Compositor compacto** junto a la mascota: «+», «Iniciar nuevo chat», micrófono (desactivado hasta la fase 7) y enviar.
5. **Chat flotante.**
   - Crece desde la burbuja cuando la respuesta pasa de unas líneas.
   - Markdown, LaTeX y código con «Copiar» (los parsers de MIKA).
   - Fuentes como íconos.
   - Esc lo cierra.
6. **Estados en vivo:** la mascota anima según los eventos del turno (pensando, trabajando, listo, error).
7. **Historial** guardado en SQLite (conversaciones y memoria).

## Se acepta cuando

- [ ] En Mac y en Windows: clic en Buddy, escribes, y el primer texto sale en menos de 1 s.
- [ ] La respuesta sale palabra por palabra (no en bloques) y el chat se expande sin saltos.
- [ ] Pedirle algo de deportes lo pasa a PARLEY (especialista de prueba) y se ve quién responde.
- [ ] Cerrar y abrir la app conserva la conversación.
