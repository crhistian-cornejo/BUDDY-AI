# PARLEY: Telegram y cuotas

PARLEY prepara las fuentes antes de cada respuesta, tanto desde el chat de Buddy como desde el bot o «Analizar» en Ajustes. El flujo vive en `core/src/parley.rs`; no depende de que el modelo invente un script ni de Context7.

## Telegram

- Consulta el estado del bot y los mensajes recibidos hoy en su chat privado vinculado. El listener existente recibe los nuevos mensajes; no se abre un segundo consumidor de Bot API. Bot API no ofrece historial retroactivo de grupos.
- La cuenta personal vuelve a consultar los grupos/canales seleccionados, comprobando su disponibilidad actual. No lee conversaciones privadas ni mensajes protegidos.
- «Hoy» usa America/Lima, desde medianoche hasta el instante de consulta. La lectura recorre el historial hacia atrás hasta medianoche, con un máximo de 200 mensajes por grupo y 10 grupos. Informa cobertura, errores y límites.
- Una lectura nueva reemplaza el snapshot anterior, incluyendo ediciones y eliminaciones. Si falla, no presenta el caché como recién leído. Las fotos válidas se adjuntan al proveedor, con un máximo combinado de 10; los medios no descargados se declaran.

## OddsPapi

Configurar en **Ajustes → Conexiones → PARLEY · Cuotas**. La clave se guarda únicamente en el llavero de macOS o el Administrador de credenciales de Windows; nunca vuelve al chat ni a SQLite. Guardarla no certifica que el plan tenga cobertura: se verifica al consultar.

El cliente usa la [API oficial v4](https://oddspapi.io/en/docs) por HTTPS, con Betano Perú (`betano.pe`). Obtiene deportes, calendario de todo el día y catálogo de mercados; solicita cuotas en un lote de hasta cinco torneos, priorizando equipos encontrados en la pregunta y los mensajes de Telegram. Conserva los eventos terminados y cancelados para distinguirlos de los pendientes.

Calendario y cuotas tienen caché de cinco minutos; catálogos, de siete días. El límite local es 20 solicitudes al día de Lima, incluidos errores; los errores esperan un minuto antes de repetirse. Hay límites de tiempo y tamaño de respuesta. Los precios solo se interpretan si el mercado y la selección están en el catálogo y el bookmaker, mercado y precio están activos. No hay operaciones para apostar ni mover dinero.

## Respuesta

El contrato de PARLEY se añade incluso si el usuario editó su definición. Separa fecha de publicación, hora real del partido y estado: pendiente, en vivo, terminado, cancelado o por confirmar. Una hora pasada por sí sola no confirma que terminó. Los parlays prematch usan pendientes de hoy y cuotas verificadas; los eventos sin cobertura requieren verificación web. El modelo debe informar fuentes, hora Lima, cuotas y límites, sin notas sobre reglas globales de programación.

Los permisos `telegram` y `cuotas` identifican este flujo nativo de PARLEY. Los ajustes explícitos del usuario prevalecen: si se quitan esos permisos, la fuente se omite y se declara. OddsPapi también requiere `web`. No hay consulta de grupos en reposo.
