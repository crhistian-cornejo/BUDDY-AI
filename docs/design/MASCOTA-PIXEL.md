# Buddy en pixel art

## Principios

- **Estilo Claude Code (Clawd) / mascotas de Codex:** pixel art chibi, pequeño, con mucha personalidad. El nivel de detalle de referencia son las mascotas de Codex (Fireball, Rocky): capucha o cabeza-disfraz, ventana de cara color piel, ojitos negros con brillo, rubor, cuerpo pequeño y sombreado con textura.
- **Buddy es «Mochi»** (elegido por el dueño el 2026-10-02): un chibi con capucha de mochi menta y un brote de dos hojas, cara crema, barriguita crema y patitas. Nada de robots.
- **Dibujado en código, no en imágenes.**
  - Cada personaje es una cuadrícula de píxeles definida como datos: filas de caracteres, donde cada carácter es un color de la paleta.
  - Se pinta con Canvas en SwiftUI (Mac) y Canvas 2D (Windows) **desde la misma definición**, así se ve igual en los dos.
  - La definición vive en `core/` y las apps la reciben.
- **Personajes propios.**
  - No se copian la mascota de Codex ni los logos de Claude, OpenAI o Google como personaje (normas de marca).
  - Cada proveedor tiene un **traje** propio: color, accesorio y un símbolo pixel inspirado en su marca. La marca oficial aparece pequeña, solo como indicador.
- **Nítido.**
  - Escala entera: cada píxel lógico mide 3, 4 o 6 puntos según el tamaño elegido.
  - Sin suavizado (`interpolation(.none)` / `imageSmoothingEnabled = false`).
- **Barato.**
  - En reposo, un fotograma quieto. Respira cada 6–10 s, ≤ 30 fps y solo unos 2 s.
  - Con «reducir movimiento», fotogramas fijos.

## Tamaño y cuadrícula

- **Cuadrícula base:** 48 × 48 píxeles por fotograma (el formato admite otros tamaños; el núcleo los valida).
- **Tamaños en pantalla:** pequeño 72 pt (×1,5, nítido en Retina), normal 96 pt (×2), grande 144 pt (×3). Están en `assets/design-tokens.json`.
- **Paleta:** como mucho 16 colores por personaje. Mochi usa 14: contorno verde oscuro, 4 tonos de menta, 2 de piel, ojos, brillo, rubor, boca y 3 de hoja.
- **Cómo se dibuja:** `scripts/characters/buddy_base.py` genera `core/characters/buddy-base.json` (lo que se distribuye) y vistas previas en PNG. Los estados nuevos se añaden ahí.

## Los personajes («trajes»)

| Traje | Cuándo | Idea |
| --- | --- | --- |
| Buddy (base, «Mochi») | Por defecto, o cuando orquesta | Chibi con capucha de mochi menta y brote de hojas |
| Claude | Responde Claude | Tonos cálidos, con un destello de ocho puntas en pixel art en el pecho |
| Codex | Responde Codex | Tonos azules de terminal, con `>_` en la pantalla de la cara |
| Gemini | Responde Gemini | Degradado azul y violeta, con una estrella de cuatro puntas pixel |
| PARLEY | Trabaja el especialista de deportes | Verde, con una gorra deportiva pixel |

Cambiar de traje es una transición de 6 fotogramas: un «parpadeo» de píxeles.

## Estados y animaciones

Siguen el modelo de las mascotas de ChatGPT/Codex (trabajando, necesita decisión, listo, bloqueado), más los propios de Buddy.

| Estado | Fotogramas | Animación | Prioridad |
| --- | --- | --- | --- |
| Reposo | 2 | Respira: el cuerpo sube 1 px cada pocos segundos | — |
| Escuchando | 4 | Ondas de sonido junto a la cabeza; la pantalla muestra barras | 1 |
| Pensando | 4 | Tres puntos en la pantalla de la cara | 4 |
| Trabajando | 6 | Teclea o «busca» (lupa pixel); línea de estado en la burbuja | 4 |
| Necesita tu decisión | 4 | Signo «?» encima, rebota | 1 |
| Bloqueado / error | 3 | Pantalla roja con «!», tiembla 1 px | 2 |
| Listo, sin leer | 4 | Salta una vez y queda con una estrella pequeña | 3 |
| Saludo | 8 | Saluda con la mano al aparecer | — |
| Arrastrado | 2 | Patas colgando | — |

**Con varias tareas a la vez**, se muestra la de mayor prioridad, como en ChatGPT: primero necesita decisión, luego bloqueado, listo y trabajando.

## Interacción

| Gesto | Qué pasa |
| --- | --- |
| Clic | Compositor compacto al lado: «+», «Iniciar nuevo chat», micrófono y enviar |
| Responde | La burbuja crece hasta un chat flotante (Markdown, LaTeX, código, copiar). Esc o clic fuera lo cierra |
| Arrastrar | Mueve la mascota; la posición se guarda por pantalla |
| Pasar el mouse | Controles debajo: lápiz (chat), micrófono, campana (actividad) |
| Clic derecho | Menú: ocultar, traje, estilo «simple» o «radial», Ajustes |
| Atajo | Opción + Espacio (Mac) / Win + Alt + B (Windows) muestra el compositor |
| «Hey Buddy» | Aparece en «escuchando» aunque esté oculta |

## Formato de datos (borrador)

```json
{
  "id": "buddy-base",
  "size": 48,
  "palette": { ".": null, "k": "#14161A", "b": "#5B7CFA", "B": "#3E5AD6", "s": "#1E2A4A", "e": "#8FF3FF" },
  "states": {
    "idle":  [ ["................................", "..........kkkkkkkkkk..........", "…"], ["…"] ],
    "think": [ ["…"], ["…"], ["…"], ["…"] ]
  },
  "fps": { "idle": 2, "think": 8, "work": 10 }
}
```

- Cada fotograma tiene `size` cadenas de `size` caracteres; `.` es transparente.
- Un test comprueba el formato: cuadrícula exacta, colores en la paleta y cada estado con sus fotogramas.

## Personajes importados (opcional)

Como [CoPet](https://github.com/ChanceYu/CoPet) y [vibe-pet](https://github.com/Seeed-Solution/vibe-pet), Buddy podrá cargar personajes externos además de los suyos:

- **El formato de las mascotas de ChatGPT/Codex:** carpeta `~/.codex/pets/<id>/` con `pet.json` y `spritesheet.webp` de 1536 × 1872 (8 columnas × 9 filas de celdas de 192 × 208; filas: idle, running-right, running-left, waving, jumping, failed, waiting, running, review). Las que hagas en Codex (`/hatch`) las podrás usar en Buddy.
- **El formato `pet.json` + `spritesheet.webp`** de CoPet y Petdex.

Las animaciones de cada formato se mapean a los estados de Buddy. Los personajes propios en código siguen siendo los de serie: pesan menos, se ven igual en las dos plataformas y no dependen de licencias de terceros.
