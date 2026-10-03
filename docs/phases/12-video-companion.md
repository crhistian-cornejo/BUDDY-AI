# Video junto a la mascota

Solicitud: ventana YouTube movible sobre la mascota sin cerrar el chat; extensión con presentación clara/oscura y logo; videos de otros sitios mediante PiP del navegador cuando esté permitido; preguntas sobre el video exclusivamente con Gemini.

Diseño: un solo reproductor oficial YouTube, con destino notch o ventana. El núcleo guarda destino y posición reproducida; nunca reproducir el mismo embed en ambas superficies. La ventana se ancla inicialmente sobre la mascota, permanece sobre otras apps y tiene zona propia de arrastre y controles fuera del video. Las preguntas mandan el contexto solo al hacer clic/enviar; no se envían frames ni navegación automáticamente.

La extensión conserva permisos limitados a YouTube. Para otros sitios el usuario elige conceder acceso; solo se registra el estado del video visible y se ofrece PiP nativo desde un gesto en el navegador. Se respetan DRM, disablePictureInPicture, políticas de permisos y anuncios. No se descarga ni captura streaming protegido. No existe garantía universal para Netflix/Disney+/Hulu/DGO/Twitch.

Gemini: la API pública admite videos públicos de YouTube y timestamps mediante entrada multimodal. El proveedor existente de Buddy usa Antigravity CLI; una URL en texto no equivale a entrada de video. No se puede afirmar un análisis visual completo sin un canal compatible. La implementación debe mostrar esta limitación y evitar que otro proveedor reciba el contexto o que una respuesta invente haber visto el video. Mantener la restricción previa de no añadir claves, pagos o servicios externos.

Fuentes: https://ai.google.dev/gemini-api/docs/video-understanding, https://developer.chrome.com/docs/web-platform/document-picture-in-picture, https://developer.chrome.com/blog/watch-video-using-picture-in-picture/, https://developer.chrome.com/docs/extensions/develop/concepts/activeTab.
