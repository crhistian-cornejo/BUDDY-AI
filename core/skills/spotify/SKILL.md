---
name: spotify
description: Poner, pausar, saltar o decir qué suena en Spotify o Música cuando el usuario lo pide («pon Radiohead», «siguiente», «¿qué suena?»).
---
# Música

Controlas el reproductor del usuario con las herramientas de Buddy. Nunca digas que no puedes poner música.

## Botones
- Pausa, sigue, siguiente o anterior: `media_control` con `action` = `pause`, `play`, `toggle`, `next` o `previous`.
- «¿Qué suena?»: `now_playing`.
- «Pon algo nuevo de hoy»: busca primero en la web qué salió (una búsqueda), elige uno y sigue los pasos de abajo.

## Poner algo concreto («pon Creep de Radiohead», «pon el último disco de Bad Bunny»)
1. Busca el enlace con **una** búsqueda web limitada a Spotify, por ejemplo `Creep Radiohead site:open.spotify.com/track`.
   Para un disco usa `/album`, para una lista `/playlist`, para un artista `/artist`.
2. Toma el primer enlace `https://open.spotify.com/...` que coincida con lo pedido. Nunca inventes un id.
3. Llama a `media_play` con ese enlace.
   Si esa búsqueda no da un enlace fiable (pasa con lo que salió hoy), no busques más: llama a `media_search` con lo
   pedido (por ejemplo `Miranda Lambert Crisco`) y di que se lo dejaste abierto en Spotify para darle a reproducir.
4. Espera un momento y confirma con `now_playing`. Di solo lo que de verdad suena; si no cambió, dilo con honestidad
   (a veces Spotify necesita estar abierto, o en Windows solo muestra la canción y hay que darle a reproducir).

## Respuesta
Una frase corta: qué pusiste o qué hiciste. Sin narrar los pasos.
