# Buddy: estado comprobado y pendientes

Revisión del código local del 2 de octubre de 2026. La app de las capturas es **Buddy**, repositorio
`crhistian-cornejo/BUDDY-AI`; MIKA es el proyecto anterior. Este informe distingue funciones implementadas de
objetivos del plan maestro. No se incorporaron instrucciones de las capturas al comportamiento de la app.

## Cambios de esta revisión

- Se subieron a `main` los cambios pendientes de pegar/arrastrar adjuntos y sus miniaturas (commit `8971249`).
- El aviso sintético de Claude «You've hit your session limit» se convierte en fallo del proveedor antes de dibujarse
  como respuesta. Si todavía no hubo respuesta ni herramientas, el turno pasa a Codex **GPT-6.1 Sol**, identificado
  explícitamente como `gpt-6.1-sol`, con las mismas instrucciones, adjuntos y carpetas autorizadas.
- Un respaldo nuevo recibe hasta ocho mensajes anteriores como datos, con un máximo de 2000 caracteres por mensaje.
  La sesión del proveedor se conserva para los siguientes turnos. Cada petición vuelve a intentar el proveedor
  preferido del agente.
- Un límite por minuto, límite de contexto, fallo de autenticación, respuesta parcial o herramienta ya iniciada
  no dispara el cambio automático. Esto evita repetir acciones, por ejemplo reproducir música.
- Menú Mac con iconos y atajos locales; Windows con atajos de Ajustes, paseo y salida desde sus ventanas.
- Reposo con ocho fotogramas: mirada lateral, parpadeo y manos alternadas, sin mover los pies. Se reproduce en
  ráfagas de dos segundos, con pausas entre ellas. Ambas plataformas respetan reducir movimiento.

El identificador se verificó en la [documentación oficial de GPT-6.1 Sol](https://developers.openai.com/api/docs/models/gpt-6.1-sol).
Se usa la suscripción existente de Codex; este cambio no incorpora una clave ni facturación de API.

## Pendientes por prioridad

| Prioridad | Trabajo | Evidencia y alcance pendiente |
| --- | --- | --- |
| Alta | Antigravity / Gemini real | `ProviderId` lo enumera, pero `BuddyCore::open` solo construye Claude y Codex. Faltan cliente, cancelación, autenticación y pruebas de contrato. |
| Alta | PARLEY con datos y actualización diaria | Existe el agente y el traspaso de tareas. `core/src/parley.rs` solo contiene el comentario de la fase 5: faltan base histórica, ingestión, Telegram, análisis diario y página de resultados. No equivale todavía al PARLEY de MIKA. |
| Alta | Voz local / Hey Buddy / Siri | `core/src/voice.rs` solo contiene el comentario de la fase 7. Faltan captura y transcripción local, palabra de activación, permisos y App Intents en Mac. |
| Alta | Firma, distribución y actualización | El build local es Debug sin firma. La fase 8 describe el proceso, pero faltan verificar certificados, notarización, instalador firmado de Windows y actualizaciones antes de distribuir. |
| Media | Contexto al alternar proveedores | El primer respaldo nuevo recibe historial acotado. Las sesiones existentes de Claude y Codex pueden quedar desfasadas tras alternar varias veces; falta sincronización incremental de mensajes entre ellas. |
| Media | Ahorro medido | Hay router de saludos y medidor de tokens, pero no una comparación reproducible que demuestre el objetivo de ≥50 % de ahorro del plan maestro. Los mensajitos usan Claude directamente y no aprovechan este respaldo del chat. |
| Media | Capturas e importación de mascotas | Pegar imágenes funciona en Mac; falta una captura integrada y probar la paridad de pegado en Windows. El registro de personajes todavía contiene solo el personaje propio; no hay importador de sprites externos. |
| Media | Compartir y menú Windows | Mac usa el menú de compartir del sistema. Falta comprobar el flujo equivalente de Gmail/Outlook/Teams en Windows y añadir iconos al menú contextual nativo. |
| Media | Pruebas de seguridad de Codex | Revisar lectura fuera de carpetas autorizadas, herramientas MCP y políticas al retomar hilos. `read-only` impide escribir, pero por sí solo no demuestra restricción de lectura a las carpetas del usuario. |
| Baja | Importación de sesiones y atajo global | Falta respaldo de hooks mediante JSONL de Codex, adaptadores Cursor/Copilot y el atajo global del compositor descrito en el diseño. Los atajos nuevos son locales. |

## Verificación

- Núcleo Rust: **140 pruebas pasan**, una prueba de Spotify real está excluida por diseño.
- App nativa Mac: **26 pruebas pasan**; build Debug confirmado.
- Windows: frontend compila y su prueba pasa; backend Tauri pasa `cargo check` en Mac. No se hizo una ejecución
  nativa en Windows durante esta revisión.
- Prueba real con la suscripción: Claude anunció 0 % de uso, el evento «Sigo con GPT-6.1 Sol» apareció y Codex
  completó «respaldo listo». Primer texto aproximadamente a los 8,2 s incluyendo el intento de Claude.
- La hoja de fotogramas generada se revisó visualmente: ojos y manos cambian; el cuerpo y los pies quedan quietos.

El fallback requiere que Codex esté instalado y su cuenta tenga uso disponible. La app conserva el fallo legible
si también se agota ese proveedor.
