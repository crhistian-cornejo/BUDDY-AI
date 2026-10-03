# YouTube detectado en el notch / barra

Implementación autorizada: 3 de octubre de 2026. Sin claves API, cuentas de desarrollador, servicios de pago ni publicación en tiendas.

- Extensión Manifest V3 incluida con Buddy para Chrome y Edge en Mac y Windows. Ajustes prepara los archivos, registra el host nativo y guía la carga local; el usuario habilita el modo desarrollador y elige la carpeta preparada.
- La extensión solo trabaja en `www.youtube.com`. Observa la URL y el estado del elemento de video local. No descarga medios, consulta endpoints privados ni guarda historial.
- Reutilizar `buddy-hook` y el socket / pipe privado por usuario para el puente nativo. Protocolo separado, mensajes acotados, identidad de extensión estable y lista explícita de orígenes. Sin un servidor de red ni secretos nuevos.
- Núcleo compartido: validar identificador, tiempos, pestaña y fuente; descartar conexiones caducadas; seleccionar la pestaña activa; mantener separado el video elegido del video detectado. Solo el clic abre reproducción.
- Reproductor oficial visible en una vista web nativa. Mantener el notch abierto durante multitarea; controles de Buddy fuera del video; cerrar o salir del modo video detiene la reproducción. Pausar la pestaña original cuando el reproductor integrado confirma reproducción. Los errores mantienen disponible la pestaña original.
- Identificar la app en las solicitudes del reproductor (`Referer` / `origin`). Respetar anuncios, restricciones de inserción y bloqueo de reproducción automática.
- Verificación: pruebas de protocolo, selección y comandos en Rust; pruebas de extensión sin datos reales; compilación de ambas interfaces. Validar en Mac la carga local y la reproducción real si el navegador está disponible. La ejecución Windows requiere un equipo Windows y se informa por separado.

Fuentes: [YouTube IFrame API](https://developers.google.com/youtube/iframe_api_reference), [requisitos del reproductor](https://developers.google.com/youtube/terms/required-minimum-functionality), [Chrome Native Messaging](https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging), [Edge Native Messaging](https://learn.microsoft.com/en-us/microsoft-edge/extensions/developer-guide/native-messaging).

## Estado de la verificación

Implementado en ambas plataformas. Pasaron cinco pruebas del núcleo (incluido el socket real), dos pruebas de la extensión, las pruebas de frames del host y dos pruebas del notch de Mac. También pasó una comprobación del ejecutable nativo con frames reales, rechazo de otro origen, filtrado de campos y desconexión. Compilación de Mac, TypeScript/Vite y Tauri en Mac correctas. Se comprobó en la app de Mac que Ajustes prepara la carpeta y muestra los pasos.

El código nuevo del núcleo y los tipos de WebView2 compilaron para Windows por separado. La compilación completa para Windows desde Mac quedó limitada por la falta del SDK de Windows; falta verificar su ejecución en Windows. El usuario instaló la extensión y se comprobó visualmente reproducción real en el notch de Mac. Tras el reinicio, Ajustes volvió a mostrar conexión y el video detectado.

Uso: Ajustes → Conexiones → YouTube → Configurar extensión. Abrir `chrome://extensions` o `edge://extensions`, activar Modo desarrollador, cargar la carpeta que muestra Buddy y recargar YouTube. No hay tienda ni registro de desarrollador. El video se abre en un reproductor oficial nuevo desde una posición aproximada; puede requerir otro clic para reproducir, mostrar anuncios o rechazar la inserción. La pestaña original se pausa cuando el nuevo reproductor confirma reproducción. Cambiar la pestaña detectada no reemplaza un video ya abierto. Cerrar el video, cambiar de sección o cerrar el notch descarga el reproductor.

Actualización 0.2: tarjeta multimedia del mismo tamaño que la de música, con miniatura pública de YouTube, título, origen, tiempo/duración y control de reproducción en la pestaña original. El botón `pip.enter` tiene tooltip «Reproducir en el notch». Mientras se ofrece YouTube, ocupa el espacio de la tarjeta de música. La duración es opcional para mantener compatibilidad con la extensión anterior. La extensión ahora lleva el logo existente de Buddy, iconos y un popup con versión, conexión y nota de privacidad. Las pruebas de Mac verifican el tamaño de la tarjeta y del reproductor. Se comprobó visualmente una reproducción real en el notch después de cerrar una segunda instancia antigua de Buddy. La actualización de los archivos requiere recargar la extensión y la pestaña.

También se comprobó visualmente la tarjeta con miniatura y tooltip. Las pruebas alojadas en la app de Mac omiten el arranque del núcleo real para que no abran otra instancia sobre la base de datos ni reemplacen el socket de la app en uso.

## Otros servicios y publicación

No se añadió acceso a Netflix, Disney+ o DGO. Una siguiente etapa puede pedir permisos opcionales por sitio y mostrar metadatos/controles del video en el navegador. Para una ventana flotante debe usar Picture-in-Picture del navegador cuando esté habilitado, respetando `disablePictureInPicture`, políticas, DRM y errores. El PiP del navegador es una ventana propia y no se puede prometer que quede insertado en el notch de Buddy. No se capturan ni extraen streams, cookies o licencias. DGO confirma acceso desde navegador, pero no encontramos documentación oficial que garantice su PiP; Disney+ tampoco garantiza esa integración.

Publicar en una tienda permite instalación normal y actualizaciones revisadas. [Chrome exige cuenta y una tarifa única](https://developer.chrome.com/docs/webstore/register/). [Edge no cobra registro para extensiones](https://learn.microsoft.com/en-us/microsoft-edge/extensions-chromium/publish/create-dev-account), pero exige cuenta de desarrollador y revisión. No se registraron cuentas ni se publicó esta extensión.

Fuentes adicionales: [restricciones y gesto de usuario del PiP](https://developer.chrome.com/blog/watch-video-using-picture-in-picture/), [limitaciones del Document PiP](https://developer.chrome.com/docs/web-platform/document-picture-in-picture), [ayuda de Netflix sobre PiP](https://help.netflix.com/es-es/node/588724338955364), [DGO y navegadores](https://www.directvgo.com/?target=EC).
