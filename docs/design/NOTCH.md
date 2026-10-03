# El notch (Mac) y la barra superior (Windows)

## Qué hace (y qué no)

**Sí**
- **Notificaciones de sesiones** de Claude Code, Codex y Gemini: «terminó en *proyecto*» o «pide permiso», con Aceptar y Rechazar ahí mismo, como Claude Island / Vibe Notch.
- **Avisos de uso:** «te queda 5 % de Claude esta semana».
- **Avisos de PARLEY:** picks nuevos, análisis del día listo.
- **Música:** carátula, título y controles; volumen del reproductor.
- **«Suelta tus archivos aquí»**, con tres opciones:
  - dárselo a Buddy;
  - Gmail, Outlook o Teams (abre el redactor con el archivo; nunca envía solo);
  - copiar la ruta.
- **Historial** de chats con Buddy (abrir uno abre la mascota).
- **Uso de los planes** de un vistazo.

**No**
- Agentes en píldoras.
- LED.
- Tips aleatorios.
- Chat dentro del notch (el chat es de la mascota).

## Comportamiento

- **Compacto.** El notch del Mac con dos «orejas»: a la izquierda la carátula o el ícono del proveedor; a la derecha un punto de estado. En Windows, una barra pequeña centrada arriba.
- **Notificación.**
  - La isla crece hacia abajo con una tarjeta: ícono del proveedor, título en una línea, detalle en una línea y botones si hacen falta.
  - Se queda 6 s, o más mientras el cursor está encima, y vuelve a compacto.
  - **Una a la vez**; las demás esperan en cola.
- **Pasar el mouse.** Se expande: música, «suelta tus archivos», historial y uso.
- **Clic** en una notificación: abre lo que corresponde (la terminal de la sesión, el chat, la página de PARLEY).

## Lo que usan las apps de notch (referencias)

| Función | Dónde se vio | En Buddy |
| --- | --- | --- |
| Música con controles | NotchNook, Alcove, Boring Notch, Venu, Notchify | Sí (de MIKA) |
| Bandeja de archivos | NotchNook, DropNotch, NotchDrop | Sí («suelta tus archivos») |
| Notificaciones y actividades en vivo | Alcove, Venu, Dynamic Edge | Sí (sesiones, uso, PARLEY) |
| Sesiones de agentes de código con aprobar/denegar | Claude Island / Vibe Notch, Venu | Sí (los hooks de MIKA) |
| HUD de volumen y brillo | Alcove | Fase posterior, si se pide |
| Portapapeles | Notchify | Fase posterior, si se pide |
| Calendario y próxima reunión | NotchNook | Fase posterior (mensajitos) |

## Estética

- **Las dos plataformas comparten** los mismos tokens (`assets/design-tokens.json`): negro de la isla, radios, tipografía del sistema, tiempos de animación y los íconos de proveedor.
- **Animación del notch en macOS.** Un único resorte de 0,38 s mueve el tamaño del fondo y la opacidad del contenido. El contenido conserva su ancho expandido mientras la forma negra lo recorta; se elimina al terminar el cierre. La reapertura interrumpe el cierre sin perder los controles. Los cambios de tamaño del reproductor siguen el mismo resorte. Con Reducir movimiento, el cambio es inmediato.
