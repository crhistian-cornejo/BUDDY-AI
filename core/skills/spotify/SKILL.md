---
name: spotify
description: Poner, pausar, saltar o decir qué suena en Spotify o Música («pon Radiohead», «pon algo nuevo de hoy», «siguiente», «¿qué suena?»).
---
# Música

Controlas el reproductor del usuario con las herramientas de Buddy. Nunca digas que no puedes poner música.
Busca siempre **dentro de Spotify**, nunca en la web: es más rápido y gasta mucho menos.

## Botones
- Pausa, sigue, siguiente o anterior: `media_control` (`pause`, `play`, `toggle`, `next`, `previous`).
- «¿Qué suena?»: `now_playing`.

## Poner algo concreto («pon Creep de Radiohead», «pon el último disco de Bad Bunny»)
1. `spotify_search` con lo pedido y el `kind` que encaje (`track` por defecto; `album`, `artist` o `playlist`).
2. Elige el resultado que mejor coincide y llama a `media_play` con su enlace `spotify:`. Nunca inventes un enlace.
3. Responde con una frase: qué pusiste. Solo si el usuario pregunta, confirma con `now_playing`.

## «Pon algo nuevo», «lo que salió hoy»
`spotify_search` con `nuevo: true` (y el género o artista si lo dijo). Elige un disco y ponlo con `media_play`.

## Si Spotify no está conectado
Si `spotify_search` dice que no está conectado, llama a `media_search` con lo pedido: se abre la búsqueda en Spotify
y el usuario le da a reproducir. Díselo en una frase y menciona que en Ajustes › Conexiones puede conectar Spotify
para que Buddy lo ponga solo. No busques en la web.

## Respuesta
Una frase corta. Sin narrar los pasos ni listar fuentes.
