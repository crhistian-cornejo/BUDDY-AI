# Letras en el notch: viabilidad y condiciones

Revisión: 3 de octubre de 2026. Estado: investigación; la función no está activada.

## Comportamiento solicitado

Detectar cualquier canción reproducida, consultar si tiene letras disponibles y ofrecer «Ver letras». Al abrirlas, sustituir los paneles inferiores por letras que avanzan con la reproducción. Al cambiar de canción, descartar el resultado anterior y consultar la nueva. Sin instalar otra app. La condición expresa del usuario es respetar las normas y los derechos sobre el contenido.

## Capacidad técnica existente

En macOS, `MediaWatcher` ya obtiene título, artista, álbum, identificador, duración, posición y estado mediante la integración local del reproductor. Actualiza el notch mientras está abierto. No es necesario un modelo de IA para detectar canciones ni sincronizar líneas.

La búsqueda tendría que ser general por metadatos, nunca por una canción fija. Una coincidencia debe comprobar versión y duración; una canción instrumental, sin letras o con una versión distinta no debe mostrar un resultado como si estuviera confirmado. La disponibilidad de un proveedor externo no demuestra que Spotify tenga letras para esa canción.

Los permisos OAuth actuales de Buddy son para crear y modificar playlists. No acreditan una autorización específica para incorporar letras.

## Restricciones verificadas

- La [política actual de Spotify](https://developer.spotify.com/policy), vigente desde el 15 de mayo de 2025, restringe integrar contenido de otro servicio (III.5), sincronizar grabaciones con medios visuales (III.6) y replicar experiencias centrales sin permiso escrito (III.11). La aplicación concreta de III.6 a un visor de letras debe confirmarse; no se asume una excepción.
- En una [respuesta de un empleado de Spotify de 2021](https://community.spotify.com/t5/Spotify-for-Developers/Question-about-developer-policy/td-p/5238042), Spotify explica expresamente que mezclar letras externas con carátulas u otro contenido de Spotify en la misma vista no está permitido. Menciona como alternativa una vista separada sin contenido procedente de Spotify. Esa respuesta antigua no acredita permiso para la sincronización solicitada bajo las condiciones actuales.
- Los [términos de Spotify](https://developer.spotify.com/terms) distinguen contenido, plataforma y aplicaciones de desarrolladores. No se presupone que leer el reproductor local constituya una excepción para Buddy, que también integra la plataforma de Spotify.
- [LRCLIB](https://lrclib.net/docs) ofrece acceso técnico a letras. No se ha verificado una licencia que autorice a Buddy a mostrar todo su catálogo. La licencia del código del servidor no acredita derechos sobre las letras alojadas. Hay una [consulta pública sobre licencias](https://github.com/tranxuanthang/lrclib/issues/111); una pregunta de un usuario no es autorización del titular.
- [Musixmatch](https://github.com/musixmatch/musixmatch-sdk) ofrece una API de letras con licencia. Antes de usarla, hay que verificar el contrato y el acceso concedidos a Buddy: catálogo, territorio, texto completo, sincronización, atribución y almacenamiento. Tener una clave por sí solo no acredita todos esos usos.

La propuesta inicial de conectar LRCLIB directamente al reproductor resolvía la viabilidad técnica, pero no estas condiciones. No se incorpora esa conexión como si estuviera autorizada.

## Implementación cuando se confirme una fuente autorizada

1. Resolver disponibilidad y coincidencias en el núcleo compartido de Rust. Presentar los mismos estados en macOS y Windows.
2. Consultar una vez por cambio de identidad de canción; cancelar la consulta anterior y rechazar respuestas tardías. Conservar resultados únicamente conforme al contrato del proveedor.
3. Diferenciar cargando, disponible, instrumental, sin coincidencia y error temporal. Un fallo de red no significa que la canción no tenga letras.
4. Abrir la vista permitida por los acuerdos aplicables y mostrar la atribución requerida. Si no se autoriza mezclar contenido, la vista deberá sustituir también el reproductor, no solo los paneles inferiores.
5. Calcular la línea activa desde la posición real y el estado de reproducción; corregir pausas, saltos y cambios de tema. No animar ni consultar mientras la vista esté oculta.
6. Probar cambios rápidos de tema, pausa, búsqueda manual de posición, instrumental, versiones distintas, ausencia de letras y errores de red. Usar texto inventado en pruebas y ejemplos; ninguna letra comercial se guarda en el repositorio.

Una demo con texto original y un reloj simulado permite comprobar el diseño y la transición sin conectarla a Spotify ni descargar letras de terceros. Esa demo necesita confirmación del usuario porque no sustituye la función real solicitada.
