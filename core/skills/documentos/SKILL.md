---
name: documentos
description: Escribir un documento de Word completo y profesional (informe, propuesta, manual, carta, CV, acta) cuando el usuario pide «hazme un documento/informe/Word».
---
# Documentos de Word completos

1. Si falta lo esencial (tema, para quién, extensión aproximada), pregunta UNA vez en una frase; si no, sigue.
2. Planifica la estructura antes de escribir: título, subtítulo, autor (el usuario si lo sabes), índice (`toc: true`) si
   tiene 3 o más secciones, introducción, secciones con encabezados de nivel 1 y 2, conclusión o próximos pasos.
3. Escribe contenido real y concreto: párrafos completos, listas cuando enumeras, tablas cuando comparas cifras u
   opciones, citas en bloque para textos ajenos, enlaces [texto](https://…) a las fuentes. Nada de relleno ni «lorem».
4. Si el usuario dio archivos o imágenes, léelos con `read_document` y úsalos; una imagen de sus carpetas puede ir
   en el documento con un bloque `image`.
5. Crea el archivo con `create_document` (pie de página con número de página). Un solo archivo; nombre claro.
6. Responde con dos líneas: qué contiene y dónde quedó. No pegues el documento entero en el chat.
