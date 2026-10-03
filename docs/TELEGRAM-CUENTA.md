# Telegram: bot y cuenta personal

Son dos conexiones independientes. El bot responde al chat privado vinculado con PARLEY: acepta texto, enlaces (también enlaces detrás de palabras), fotos con o sin descripción e imágenes JPEG/PNG/WebP/GIF enviadas como archivo, hasta 10 MB. Las imágenes se validan y reducen antes de enviarlas al proveedor con visión; las copias quedan en el historial de ese chat. Los enlaces se consultan con las herramientas web del agente, si están habilitadas. Otros archivos, voz y video no se procesan. El bot no hereda las membresías de la cuenta. La cuenta personal consulta grupos y canales que ya pertenecen al usuario, y únicamente los que elige en Ajustes → Conexiones → Telegram · Cuenta personal.

## Acceso

1. En https://my.telegram.org/apps, crear una aplicación y obtener api_id y api_hash. Pegar ambos directamente en Ajustes, nunca en el chat de Buddy.
2. Escribir el teléfono con prefijo de país y pedir el código. Escribir el código recibido en Telegram y, si se requiere, la contraseña de verificación en dos pasos.
3. Pulsar Elegir grupos y seleccionar hasta diez.
4. Consultar mensajes trae los últimos veinte por grupo. El caché conserva hasta doscientos mensajes; la lista muestra los treinta más recientes. No es una sincronización completa del historial, ni una escucha automática.
5. Analizar con PARLEY manda hasta treinta mensajes más recientes (con un límite de texto de aproximadamente 24 KB) y hasta diez fotos al proveedor del agente; el resultado queda también en el chat Telegram · Grupos. La lectura no clasifica automáticamente cada mensaje como apuesta. El análisis conserva fuente y fecha y debe reconocer datos ausentes.

El lector usa MTProto (grammers), portado de MIKA bajo MIT, desde el núcleo compartido. No envía mensajes, se une a grupos ni marca como leído; excluye chats privados, grupos/canales protegidos y mensajes protegidos. Solo descarga fotos JPEG de Telegram, de hasta 5 MB, comprobadas; no descarga documentos ni ejecuta enlaces. Si no hay un proveedor con visión, las fotos requieren revisión manual.

Credenciales y claves de sesión viven exclusivamente en Llavero/Credential Manager. El teléfono, código y contraseña no se persisten. Selección, textos y metadatos se guardan localmente en SQLite; fotos dentro de la carpeta de datos de Buddy. Cambiar la selección elimina del caché los mensajes y fotos de grupos retirados. Cerrar sesión elimina el caché y las claves locales, y solicita también el cierre remoto; si este falla se debe revocar el dispositivo desde Telegram. Los análisis ya guardados son chats independientes y permanecen en el historial.

No hay tráfico ni consumo de modelos por sondeos en reposo. Las operaciones se serializan fuera del hilo de interfaz, con un límite de 90 segundos por petición. En Mac y Windows se inicia sesión por separado con la misma cuenta; no hace falta crear otro bot ni copiar una sesión entre dispositivos.

Fuentes: https://core.telegram.org/api/obtaining_api_id · https://core.telegram.org/api/auth · https://core.telegram.org/api/content-protection
