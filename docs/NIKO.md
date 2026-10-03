# Niko · Finanzas

Niko registra gastos, ingresos, pagos y vouchers en las mismas bases de Notion desde Claude o GPT. Al agotarse una suscripción, el router continúa con la otra. Usa los conectores existentes de claude.ai y las apps nativas de ChatGPT a través de Codex; no requiere una API key ni crear un proyecto en Google Console.

## Funcionamiento del router

- Chat, Telegram y revisión del correo permiten Claude y GPT. Gemini se excluye de los turnos que necesitan estas cuentas.
- Revisiones y chat: Haiku o GPT Luna, con esfuerzo bajo. Preparación inicial de Notion: Sonnet o GPT Sol.
- La cuota conocida se comparte entre chat y tareas de fondo. Se omite una ruta agotada hasta su reinicio; si el proveedor no indica cuándo, se vuelve a probar tras 15 minutos. No se cambia por una simple limitación temporal de peticiones.
- Ambos conservan la página, bases, claves de movimiento y hora de la petición. Si la cuota se agota después de usar herramientas, el siguiente proveedor debe comprobar lo ya escrito antes de continuar. Las respuestas parciales del chat de Niko no se muestran como confirmaciones.
- Gmail y Drive se ofrecen solo para leer. Notion permite las herramientas de páginas, bases, vistas y consultas que Niko necesita. En GPT, las demás apps y herramientas quedan desactivadas en el turno; los permisos usan los nombres completos del catálogo nativo, no solo los nombres cortos mostrados por app/read.
- Se comprueba la disponibilidad efectiva de las herramientas antes del turno de GPT. Las credenciales siguen administradas por Claude/ChatGPT; Buddy no las copia.
- Cada revisión procesa como máximo 15 mensajes nuevos. Con destinos ya guardados empieza por Gmail; si la búsqueda falla, se detiene sin lecturas ni escrituras adicionales en Notion. Las consultas de Notion admiten hasta 100 filas por petición y los informes deben paginar para obtener el total.
- Los cálculos del dashboard y los presupuestos los hace el núcleo. Las conexiones de fondo se cierran al terminar, sin cerrar los otros chats. Las revisiones y actualizaciones manuales del dashboard no se solapan.

La lógica compartida vive en core/src/account_router.rs, core/src/accounts.rs, core/src/niko.rs y el proveedor nativo de Codex. Mac y Windows muestran «Claude y GPT · cambio automático» en Ajustes › Agentes › Niko.

## Uso

1. Inicia sesión en Claude Code y/o Codex con tus suscripciones. Las cuentas de cada proveedor son independientes: una conexión en ChatGPT no autoriza por sí sola la de Claude.
2. Conserva tus conexiones de Gmail, Drive y Notion en Claude o ChatGPT. No hace falta recrear las bases al cambiar de proveedor.
3. En Ajustes › Agentes › Niko, pulsa «Volver a comprobar», guarda la página de Notion y pulsa «Revisar ahora». Estar en la lista de apps conectadas no confirma todos los permisos de Gmail; la revisión comprueba el acceso real.
4. Activa la revisión automática en un solo equipo cuando el acceso al correo esté disponible. Se puede programar cada 10, 20, 30 o 60 minutos. Inicialmente está apagada.

## Verificación real · 3 de octubre de 2026 (Lima)

Se probó el router real con las suscripciones existentes: Claude devolvió cuota agotada hasta la 1:10 a. m.; GPT Luna continuó y ejecutó notion.fetch sobre la página del usuario. Después, el núcleo leyó las bases, calculó el dashboard y lo actualizó con notion.notion-update-page; se verificó la escritura con otra lectura. No se inventaron operaciones ni una puntuación financiera.

Los destinos existentes son:

- [Movimientos](https://app.notion.com/p/00000000000000000000000000000001)
- [Presupuestos](https://app.notion.com/p/00000000000000000000000000000002)
- [Dashboard](https://app.notion.com/p/00000000000000000000000000000003)

**Notion funciona por la ruta de GPT. El bloqueo pendiente es Gmail:** tanto gmail.search_emails como gmail.search_email_ids devolvieron ACCESS_TOKEN_SCOPE_INSUFFICIENT. Hace falta renovar la autorización de Gmail en ChatGPT para conceder lectura. No se necesita Google Console y este error no implica que Notion esté desconectado.

También se ejecutó la revisión con los ajustes reales de Buddy: guardó ese error, conservó la ventana pendiente y no importó correos. Con la instrucción de detenerse tras el fallo, solo llamó a la búsqueda de Gmail y no consultó ni escribió en Notion. La revisión automática permanece apagada.

Las pruebas cubren el cambio Claude → GPT, GPT → Claude después de una escritura parcial, objetivos y hora compartidos, pausas de cuota y recuperación, permisos y errores de conexión que no se deben mostrar como éxito. La compilación Debug de macOS y la compilación web de Windows pasaron. La suite completa detectó un fallo ajeno al router en el accesorio «gorra» del avatar; el resto pasó. La app nativa de Windows no se ejecutó en este Mac.
