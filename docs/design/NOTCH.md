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
| HUD de volumen y brillo | Atoll, Alcove | macOS: estados compactos y sustitución de teclas con Accesibilidad |
| Portapapeles | Atoll, Notchify | Textos guardados explícitamente |
| Calendario y próxima reunión | Atoll, NotchNook | Agenda local .ics elegida por el usuario |

## Estética

- **Las dos plataformas comparten** los mismos tokens (`assets/design-tokens.json`): negro de la isla, radios, tipografía del sistema, tiempos de animación y los íconos de proveedor.
- **Animación del notch en macOS.** Un único resorte de 0,38 s mueve el tamaño del fondo y la opacidad del contenido. El contenido conserva su ancho expandido mientras la forma negra lo recorta; se elimina al terminar el cierre. La reapertura interrumpe el cierre sin perder los controles. Los cambios de tamaño del reproductor siguen el mismo resorte. Con Reducir movimiento, el cambio es inmediato.

## Mejoras inspiradas en Atoll

Referencia de interacción: [Atoll](https://github.com/Ebullioscopic/Atoll). Implementación propia sobre las herramientas de Buddy, conservando avisos, permisos, música, atajos, archivos y uso.

- Encabezado con la actividad actual, botón **Mantener abierto** y cierre con **Esc**. Retirar el cursor conserva las herramientas mientras estén fijadas; cerrar nunca responde un permiso ni descarta archivos.
- Actividades compactas con la misma prioridad en Mac y Windows: una sesión que espera respuesta, Buddy respondiendo, una sesión trabajando, enfoque y música.
- El enfoque mantiene los accesos de 25 y 50 minutos y añade un selector de 5, 15, 25, 50, 90 y 120 minutos.
- Tarjetas con bordes tenues, contorno discreto y sombra exterior en Mac; gradiente tenue dentro de las tarjetas en Windows.
- En Windows, apertura tras 120 ms y cierre tras 300 ms fuera del área. El avance de la música y el tiempo restante se actualizan sin reconstruir botones ni interrumpir el foco del teclado.
- Las novedades no aparecen en el notch ni en la barra. El servicio de mensajitos sigue disponible para Buddy fuera de esa superficie.
- El uso ocupa una sola fila de iconos y porcentajes usados: Claude y Gemini prefieren 5 h; Codex prefiere la semana. Si falta esa ventana, se muestra otra disponible. El tooltip indica proveedor, ventana y reinicio; se omiten proveedores sin ventanas.

### Funciones de Atoll que encajan con Buddy

Implementadas en ambas plataformas:

| Prioridad | Adaptación para Buddy | Motivo |
| --- | --- | --- |
| 1 | Bandeja de archivos que conserve lo soltado y permita añadir o retirar archivos individualmente | Extiende «Dárselo a Buddy», compartir y copiar rutas sin ocupar el panel de herramientas todo el tiempo. |
| 2 | Widgets elegibles en una pestaña de utilidades | Permite añadir funciones conservando el tamaño compacto; Atoll distribuye sus herramientas en pestañas. |
| 3 | Portapapeles con elementos guardados explícitamente | Recuperar textos, enlaces o fragmentos y llevarlos al chat; adaptar el historial de Atoll a las tareas de Buddy. |
| 4 | Batería/carga y próxima cita como widgets opcionales | Datos breves y útiles; la agenda debe indicar su fuente y abrir el evento correspondiente. |

### Bandeja y utilidades

- Accesos compactos en la cabecera, como en MIKA: casa para **Inicio**, **+** para **Archivos** y cuadrícula para **Utilidades**. En Mac quedan a la altura del notch físico: pestañas a la izquierda, centro reservado para la cámara y fijar/cerrar más widgets a la derecha. La selección tiene fondo de cápsula; el tooltip y la etiqueta accesible indican cada nombre. El contenido empieza bajo el notch con su margen original de 16 puntos. El fondo es negro sRGB puro, sin borde gris, y la zona de cámara se cubre permanentemente durante la animación. Inicio conserva música, agentes, enfoque y atajos; el pie con uso de planes está disponible en las tres.
- La bandeja guarda referencias a un máximo de 32 archivos o carpetas. Añadir o soltar acumula sin duplicar. Retirar solo quita la referencia; cerrar, copiar o llevar al chat conserva la bandeja. Los archivos que ya no existen siguen visibles para poder retirarlos. Mac añade Compartir; Windows ofrece mostrar cada archivo en el Explorador.
- El portapapeles guarda únicamente al pulsar **Guardar actual**, hasta 20 textos. Permite copiar, retirar o añadir al borrador de Buddy sin enviarlo. Guarda el texto exacto y no registra cambios del portapapeles.
- **Widgets** permite mostrar u ocultar batería, próxima cita y portapapeles; la selección se conserva entre reinicios.
- Batería usa los datos del sistema y distingue carga, corriente y batería. Un equipo sin batería lo indica.
- Próxima cita lee un archivo local **.ics** elegido explícitamente. Muestra su nombre o el de la agenda, respeta zonas IANA, eventos de todo el día, RRULE/RDATE/EXDATE y excepciones de una instancia. Busca repeticiones hasta un año por delante, con un máximo de 512 resultados por serie; agendas con zonas personalizadas o cambios RANGE muestran un error para volver a exportarlas. Abre el enlace del evento si existe; de lo contrario, exporta la instancia para abrirla en la aplicación de calendario. No sincroniza una cuenta de calendario.
- Batería y agenda se actualizan cada 30 segundos solo mientras Utilidades esté visible. La lectura de la agenda se hace fuera de la interfaz. Los datos elegidos se guardan localmente en `notch.tools` dentro de la base de Buddy.

### Estados compactos del sistema (macOS)

- Volumen y brillo ocupan las alas del notch cerrado: icono y nombre a la izquierda, barra a la derecha. Duran 2,4 segundos; cada cambio sustituye al anterior y reinicia ese tiempo. El tooltip indica el porcentaje y, para audio, la salida actual. Si las herramientas están abiertas, el estado aparece en su encabezado.
- Con Accesibilidad, Buddy intercepta únicamente las teclas de volumen, silencio y brillo que puede controlar. CoreAudio ajusta el volumen; el brillo integrado se resuelve en tiempo de ejecución mediante servicios de pantalla de macOS, que Apple no ofrece como API pública moderna. Un dispositivo sin control compatible conserva sus teclas nativas. Opción+Mayúsculas conserva los pasos finos; los demás atajos y las teclas de reproducción siguen funcionando normalmente. No se suspende ni modifica el servicio de indicadores del sistema.
- Bluetooth muestra conexión/desconexión con el nombre e icono de AirPods, auriculares, periféricos u otros dispositivos. El anillo con marca indica conexión; no representa una batería desconocida. También se avisa al cambiar de salida de audio. Los dispositivos ya conectados al iniciar no generan avisos.
- No molestar / Concentración muestra luna y On/Off al cambiar el estado. Usa el estado compartido de macOS con autorización; puede leer el estado local compatible si ya es accesible. Si no hay una fuente disponible, no inventa un estado Off. No solicita acceso total al disco.
- **Ajustes → General → Notch · estados del sistema** permite activar cada grupo y abrir la configuración de Accesibilidad o solicitar el estado de concentración. También está disponible en el menú Widgets. Las elecciones persisten en `notch.system.*`.
- Los avisos normales de Buddy permanecen compactos en ambas plataformas; pasar el cursor abre la tarjeta completa. Los permisos pendientes conservan su tarjeta con las acciones. Estos avisos corresponden a Buddy y sus agentes; no se captura el Centro de notificaciones de otras aplicaciones.
