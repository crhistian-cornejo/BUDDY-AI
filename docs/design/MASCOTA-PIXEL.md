# Buddy en pixel art

## Principios

- **Estilo Claude Code (Clawd) / mascotas de Codex:** pixel art chibi, pequeño, con mucha personalidad. El nivel de detalle de referencia son las mascotas de Codex (Fireball, Rocky): capucha o cabeza-disfraz, ventana de cara color piel, ojitos negros con brillo, rubor, cuerpo pequeño y sombreado con textura.
- **Buddy** (elegido por el dueño el 2026-10-02) es un chibi con capucha de mochi menta y un brote de dos hojas, cara crema, barriguita crema y patitas. Nada de robots.
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
- **Paleta:** como mucho 16 colores para el personaje, y hasta 24 contando sus objetos. Buddy usa 16 (contorno verde oscuro, 4 tonos de menta, 2 de piel, ojos, brillo, rubor, boca y 3 de hoja) más 4 de la laptop (plata, brillo, sombra de la bisagra y la luz de la pantalla en la cara: `X x h e`).
- **Cómo se dibuja:** `scripts/characters/buddy_base.py` genera `core/characters/buddy-base.json` (lo que se distribuye) y vistas previas en PNG. Los estados nuevos se añaden ahí.

## Los personajes («trajes»)

| Traje | Cuándo | Idea |
| --- | --- | --- |
| Buddy (base) | Por defecto, o cuando orquesta | Chibi con capucha de mochi menta y brote de hojas |
| Claude | Responde Claude | Tonos cálidos, con un destello de ocho puntas en pixel art en el pecho |
| Codex | Responde Codex | Tonos azules de terminal, con `>_` en la pantalla de la cara |
| Gemini | Responde Gemini | Degradado azul y violeta, con una estrella de cuatro puntas pixel |
| PARLEY | Trabaja el especialista de deportes | Capucha cielo, con una gorra deportiva pixel |

Cambiar de traje es una transición de 6 fotogramas: un «parpadeo» de píxeles.

### Caras de los agentes (2026-10-02)

Cada agente lleva su propia cara: el mismo Buddy con otra capucha, un accesorio y otros ojos (`core/src/look.rs`).

- **Color:** menta (el de Buddy), cielo, lavanda, rosa, fresa, mandarina, limón y grafito. Cada uno saca sus 5 tonos
  (contorno, sombra honda, sombra, base, luz) con los mismos saltos de tono, saturación y luz que la menta.
- **Accesorio:** ninguno, gorra, lentes, audífonos, corona, bandana o gorro de lana. Son sellos de píxeles anclados a
  la ventana de la cara, así siguen al cuerpo en todos los estados; los sombreros quitan el brote.
  El color del accesorio es la «insignia» (o uno automático que resalta sobre la capucha).
- **Ojos:** normales, felices (^ ^), serios (cejas rectas) o guiño. Solo cambian los ojos abiertos; parpadeos y miradas
  quedan como están.
- **De dónde sale:** `cara: color=…, accesorio=…, ojos=…, insignia=…` en el agent.md y, por encima, el ajuste
  `agent.<id>.cara` (mismo `k=v` o JSON) que escribe Ajustes › Agentes. Sin `cara:`, un color según el id.
- Buddy por defecto es buddy-base byte a byte; PARLEY lleva capucha cielo y gorra fresa.
- Vistas previas: `BUDDY_LOOK_PREVIEW=<carpeta> cargo test -p buddy-core look::tests::preview_sheets -- --ignored`
  (también escribe el `agent-looks.json` de `apps/windows/preview`).

## Estados y animaciones

Siguen el modelo de las mascotas de ChatGPT/Codex (trabajando, necesita decisión, listo, bloqueado), más los propios de Buddy.

| Estado | Fotogramas | Animación | Prioridad |
| --- | --- | --- | --- |
| Reposo | 8 | Mueve una mano cada vez, mira a los lados y parpadea; cuerpo y pies quietos | — |
| Escuchando | 4 | Ondas de sonido junto a la cabeza; la pantalla muestra barras | 1 |
| Pensando | 4 | Tres puntos en la pantalla de la cara | 4 |
| Trabajando | 6 | Teclea o «busca» (lupa pixel); línea de estado en la burbuja | 4 |
| Necesita tu decisión | 4 | Signo «?» encima, rebota | 1 |
| Bloqueado / error | 3 | Pantalla roja con «!», tiembla 1 px | 2 |
| Listo, sin leer | 4 | Salta una vez y queda con una estrella pequeña | 3 |
| Saludo | 8 | Saluda con la mano al aparecer | — |
| Arrastrado | 2 | Patas colgando | — |

**Con varias tareas a la vez**, se muestra la de mayor prioridad, como en ChatGPT: primero necesita decisión, luego bloqueado, listo y trabajando.

### Estados dibujados (2026-10-02)

`idle`, `blink`, `look`, `wave`, `walk-right`, `walk-left`, `drag`, `think`, `work`, `ask`, `error`, `done` y `sleep`.
Sentado y aburrido: `sit-down` (3), `sit` (1, fijo), `sit-blink` (2), `sit-look` (4), `sit-yawn` (5), `sit-swing` (7) y
`stand-up` (2). Para dormir: `lie-down` (3), `sleep` (8 a 2 fps), `sleep-still` (1) y `wake-up` (5).
Las hojas de cada estado salen con `scripts/characters/buddy_base.py --preview DIR`.

### Con lentes y laptop (2026-10-02)

Cuando Buddy piensa o trabaja (el chat respondiendo, una tarea de un agente) se pone lentes, se sienta, saca una
laptop plateada y teclea; al terminar la cierra, se quita los lentes y se levanta (y reacciona si hay `done`/`error`).

- Estados: `laptop-on` (4 fotogramas a 8 fps: se agacha con lentes, aparece la laptop cerrada, la tapa a medias, y
  termina en el primer fotograma de `laptop-type`), `laptop-type` (6 a 6 fps, `work`), `laptop-think` (4 a 3 fps,
  `think`) y `laptop-off` (3 a 8 fps: tapa a medias, cerrada, se levanta). Las apps traducen `think`/`work` a estos;
  `ask` y `listen` siguen igual y primero guardan la laptop.
- **La laptop (2026-10-03):** de frente, Buddy sentado detrás de una laptop estilo MacBook vista por detrás: tapa
  plateada de 18 × 11 con esquinas redondeadas, brillo arriba y a la izquierda, un destello en diagonal, bisagra más
  oscura y una base fina un píxel más ancha. **La tapa va lisa: sin manzana, sin logo, sin punto.** Las patitas se
  ven a los lados de la tapa (la que teclea sube una fila); al pensar, una patita sostiene la barbilla.
- **Luz de pantalla:** la parte baja de la cara toma un azul frío (`e`): una fila normalmente, dos cuando parpadea
  la pantalla (`laptop-type`, fotograma 3) o pulsa (`laptop-think`); con dos, también los brillos de los lentes.
- Teclear: patitas alternadas, un cabeceo de un píxel, un parpadeo de la luz y una pausa con los ojos entornados.
- Se probó también una vista de perfil (Buddy de lado, pantalla en ángulo): a 96 pt la laptop queda en una cuña de
  15 columnas que no se lee, y un perfil de verdad pide otra cabeza, que rompe el ancla de la cara de `look.rs`
  (lentes, sombreros, ojos). Se quedó la vista de frente.
- Las caras de los agentes (`look.rs`) no tocan los colores de la laptop (hay prueba). Con «reducir movimiento» se
  muestra el fotograma fijo de teclear, sin transiciones; fuera de `think`/`work` no corre nada.

### Vida en reposo (núcleo: `core/src/pet.rs`)

- **`PetBrain` decide** cada 4–9 s: respirar, parpadear, mirar a los lados, saludar o pasear (22 %).
  - **Pasea** 24–180 pt a 32 pt/s, sin salir del ancho útil de la pantalla.
  - Cerca de un borde **se da la vuelta**; si no hay sitio, hace otra cosa.
- **Duerme** tras unos 10 min esperando sentado sin usarlo (610 s desde el último uso, incluidos los 10 s antes
  de sentarse), aunque sigas usando el teclado en otra app. Se acuesta de lado en 3 fotogramas y respira a 2 fps:
  salen tres «zzz» de la boca y una burbujita de moco se infla y desinfla. Sigue acostado entre ciclos.
  Pasar el cursor, hacer clic, abrir el chat o empezar una tarea lo despierta; `wake-up` termina de pie.
  Con «reducir movimiento», mantiene `sleep-still` y omite las transiciones y efectos animados.
- **«Reducir movimiento»:** solo parpadea.
- **«Pasear por la pantalla»** se desactiva desde el clic derecho.
- **Al soltarla** después de arrastrarla, `clamp_to_area` la devuelve entera dentro del área útil (bordes y esquinas, bajo la barra de menús y sobre el Dock o la barra de tareas).
- **Se sienta, aburrido,** tras 10 s sin usarlo (`SIT_AFTER_SECONDS`): sin pasar el ratón, clic ni arrastre, con el
  chat cerrado y sin trabajar, hablar ni pasear.
  - Se sienta en 3 fotogramas (375 ms) y queda en el fotograma fijo `sit` (párpados caídos).
  - Cada 5–10 s, un gesto corto que vuelve a `sit`: parpadeo lento, mirar de lado, balancear un pie o bostezar
    (un bostezo cada ~30 s). Sentado también puede levantarse a pasear.
  - Cualquier uso (pasar el ratón, clic, abrir o cerrar el chat, un estado de trabajo, el globo) lo levanta en
    2 fotogramas. Sentado nunca bloquea clics ni arrastre.
  - Con «reducir movimiento»: solo el fotograma fijo sentado, sin transiciones ni gestos.
  - El plan del núcleo trae `intro` (transición previa) y `rest` (`idle`, `sit` o `sleep-still`, el fotograma en el que queda).
- **Entre planes no corre nada.**

Actualización 2026-10-02: `idle` dura 2 s a 4 fps y alterna ojos y manos sin desplazar la ventana. Mac y Windows
usan los mismos ocho fotogramas. Con «reducir movimiento» las interfaces muestran un fotograma fijo por estado.
En Mac una reproducción nueva interrumpe la anterior para que el reposo no tape el estado de trabajo.

### Menú y atajos locales

En Mac: historial ⌘F, nuevo chat ⌘N, cerrar chat ⌘W/Esc, Ajustes ⌘,, pasear ⌘⇧P y salir ⌘Q.
Los atajos funcionan mientras una ventana de Buddy tiene el foco. El menú incluye iconos para historial,
novedades, pasear, Ajustes y salir. Windows usa Ctrl en lugar de ⌘ (historial/nuevo chat en el chat).
Los iconos del menú contextual nativo de Windows quedan pendientes; conserva las etiquetas y la marca de paseo.

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
