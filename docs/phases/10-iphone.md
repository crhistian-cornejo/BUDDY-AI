# Buddy en el iPhone: especificación

2026-10-03 · Diseño aprobado en conversación por el dueño; este documento es lo que se construye.

## Objetivo

Que el dueño, fuera de casa o de la oficina, siga siendo productivo con Buddy desde su iPhone 17 Pro: ver qué pasa
en su Mac y en la PC del trabajo, recibir avisos, chatear con Buddy y los especialistas (también por voz), y aprobar
o rechazar lo que sus agentes de código le piden. Con la misma estética y la misma mascota que en el escritorio.

Lo que dijo el dueño:
- Sin pagar más por los modelos: se usan las suscripciones de siempre.
- App nativa (acepta la cuenta de Apple Developer, 99 USD al año).
- Conectarse tanto a la Mac como a la PC con Windows; cada una con su historial, porque las bases son locales.
- Notificaciones, historial, chat, voz, Face ID.
- La mascota en la app, en la Isla Dinámica y en la pantalla bloqueada; tocarla abre el chat.
- Muy seguro, sin filtrar nada; auditoría de seguridad en Mac, Windows e iPhone.

Lo que se asume:
- El iPhone no ejecuta modelos. Todo turno corre en una máquina encendida, con la suscripción de esa máquina.
- Un solo iPhone por máquina en esta fase.
- El relé es personal: lo despliega el dueño en su propia cuenta gratuita de Cloudflare.

## Límites que conviene saber antes

- **Máquina apagada o dormida:** el iPhone muestra su historial guardado y nada más. No hay cola de mensajes para
  después. Para tener Buddy siempre disponible, la Mac de casa debe quedar enchufada y sin dormir.
- **La mascota fuera de la app no puede animarse de forma continua.** iOS no deja correr código ni animaciones
  libres en la Isla Dinámica ni en la pantalla bloqueada. Lo que sí se puede: la mascota **cambia de pose con cada
  novedad** (pensando, trabajando, saludando porque te espera, contenta porque terminó, dormida), con la transición
  del sistema. Dentro de la app se anima completa, como en el escritorio.
- **La Isla y la tarjeta de la pantalla bloqueada aparecen mientras pasa algo** (una respuesta en curso, una sesión
  trabajando o esperándote, el temporizador de concentración) y iOS las retira a las pocas horas. La presencia fija
  es un widget de pantalla bloqueada con la mascota en reposo.
- **PC del trabajo:** lo que salga de ella llega cifrado a un teléfono personal. Que eso sea aceptable para la
  empresa lo decide el dueño.

## Piezas

```
iPhone ── WSS ──┐
                ├── relé (Cloudflare Worker): reenvía bytes cifrados y entrega los avisos a Apple (APNs)
Mac ──── WSS ───┤
PC ───── WSS ───┘        cada máquina con su iPhone forma una «sala»; el iPhone tiene una sala por máquina
```

| Pieza | Ruta | Qué hace |
| --- | --- | --- |
| `buddy-remote` | `remote/` (crate nuevo) | Emparejado, cifrado y formato de los mensajes. Lo usan el núcleo y la app de iPhone (UniFFI), para que el código de seguridad exista una sola vez. |
| Módulo remoto | `core/src/remote/` | Conexión con el relé, lista blanca de llamadas, reenvío de eventos, reglas de avisos, registro de acciones remotas. |
| Relé | `relay/` (TypeScript) | Un Durable Object por sala con WebSockets en hibernación. No guarda mensajes. |
| Ajustes → iPhone | `apps/macos`, `apps/windows` | QR para emparejar, estado, «Olvidar iPhone», interruptores, registro de acciones. |
| App de iPhone | `apps/ios/` (SwiftUI, XcodeGen) | Interfaz. Tres destinos: la app, los widgets con la actividad en vivo, y la extensión que descifra avisos. |

La app de iPhone es una tercera interfaz, como la de Windows: recibe los mismos registros JSON que hoy recibe el
frontend de Windows y dibuja los mismos eventos. No decide proveedores, datos ni reglas.

## Emparejado

1. En el escritorio, Ajustes → iPhone → «Emparejar». La primera vez pide la dirección del relé y su clave de dueño
   (se guarda en Llavero / Credential Manager). El núcleo crea la sala en el relé.
2. Muestra un QR válido 5 minutos y de un solo uso: dirección del relé, identificador de sala (128 bits al azar),
   llave de acceso a la sala, clave pública de la máquina y un secreto de emparejado. El núcleo genera la matriz del
   QR y cada app la pinta.
3. El iPhone lo escanea (tras Face ID), se conecta a la sala y envía su clave pública autenticada con el secreto del
   QR. Como el secreto viajó por la cámara y no por la red, el relé no puede hacerse pasar por ninguno de los dos.
4. La máquina muestra «iPhone de … emparejado». Desde ahí, las dos claves públicas quedan fijadas.

Desemparejar: «Olvidar iPhone» en el escritorio u «Olvidar esta máquina» en el iPhone. Borra claves y copia local, y
pide al relé borrar la sala. Si el teléfono se pierde, basta «Olvidar iPhone» en cada máquina.

## Cifrado

- **Conexión:** protocolo Noise, patrón `KK` (`Noise_KK_25519_ChaChaPoly_SHA256`), con la biblioteca `snow`. Las dos
  partes ya conocen la clave fija de la otra por el emparejado; cada conexión añade claves efímeras, así que grabar
  el tráfico y robar una clave después no descifra lo pasado. No se inventa criptografía propia.
- **Avisos (push):** llegan con la app cerrada, sin conexión Noise. Se cifran con una clave de avisos derivada en el
  emparejado (HKDF), XChaCha20-Poly1305 con nonce al azar, contador y hora para descartar repeticiones. La extensión
  de notificaciones los descifra en el iPhone. Apple y el relé solo ven bytes cifrados; si el descifrado falla se
  muestra un texto genérico («Buddy · Novedad en tu Mac»).
- **Claves:** en Llavero (Mac), Credential Manager (Windows) y Llavero del iPhone, marcadas «solo este dispositivo»
  (no viajan a iCloud ni a copias de seguridad). La clave de avisos del iPhone es legible tras el primer desbloqueo;
  la clave de conexión, solo con el teléfono desbloqueado.
- **Lo que el relé sí ve:** que existe la sala, cuándo y cuánto se habla, direcciones IP y el identificador de
  avisos del iPhone. Puede retrasar o tirar mensajes; no leerlos ni falsificarlos.

## Qué puede pedir el iPhone

Dentro del canal cifrado viajan llamadas `{id, call, args}` con su respuesta, y eventos. El núcleo atiende solo esta
lista; cualquier otra llamada se rechaza y se anota.

| Leer | Hacer |
| --- | --- |
| `hello` (nombre de la máquina, plataforma, versión) | `send_message` (texto e imágenes) |
| `chats`, `search_chats`, `messages`, `image_preview` | `cancel_chat`, `regenerate`, `remove_queued` |
| `agents`, `agent_sprite`, `sprite`, `chat_commands`, `chat_suggestions` | `answer_approval` (permitir o rechazar, una vez) |
| `queued_messages`, `sessions`, `usage`, `briefing` | `focus_start`, `focus_stop` |
| `focus_status`, `voice_vocabulary` | `register_push` (identificadores de avisos del iPhone) |

Eventos que se reenvían: `ChatStarted`, `ChatDelta`, `ChatTool`, `ChatSource`, `ChatActivity`, `ChatDone`,
`ChatFailed`, `ChatQueueChanged`, `ChatDequeued`, `MascotState`, `SessionUpdate`, `ApprovalRequest`,
`ApprovalClosed`, `UsageChanged`, `UsageLow`, `BriefingReady`, `FinanceRecorded`, `BudgetAlert`, `FocusChanged`,
`FocusFinished`. No se reenvían `SettingChanged`, `MediaCommand`, `ScreenshotRequest`, `TelegramChanged` ni
`NikoChanged`.

Reglas:
- **El iPhone nunca** lee ni cambia ajustes, claves, carpetas autorizadas, hooks, permisos de agentes ni reglas de
  «Permitir siempre»; no borra chats; no ve rutas de archivos fuera de las miniaturas de adjuntos.
- **Turnos iniciados desde el iPhone:** mismos permisos del agente que en el escritorio, salvo capturas de pantalla,
  que se niegan. Lo que en el escritorio pide un clic, pide una tarjeta de permiso, que llega también al teléfono.
- **Permisos:** «Permitir» desde el iPhone exige Face ID cada vez. «Permitir siempre» solo existe en el escritorio.
  Un interruptor en Ajustes → iPhone («Aprobar permisos desde el iPhone», encendido) lo reduce a solo rechazar.
- **Adjuntos:** solo imágenes (JPEG, PNG, WebP, GIF), hasta 10 MB, decodificadas con límites y reducidas con el
  mismo código que ya usa Telegram; se copian a la carpeta de adjuntos de Buddy.
- **Registro:** cada acción remota («iPhone envió un mensaje», «iPhone permitió: git push») queda en una lista de
  las últimas 200, visible en Ajustes → iPhone.

## Reposo y consumo

- El módulo remoto no existe hasta que hay un iPhone emparejado. Con uno emparejado mantiene una conexión saliente
  bloqueada en lectura (sin sondeos; reintentos 2 s, 4 s, 8 s… hasta 5 min, como Telegram).
- El relé avisa si el iPhone está conectado. Sin iPhone conectado no se envía ningún evento, solo avisos.
- Con iPhone conectado, `ChatDelta` se agrupa a un máximo de cinco envíos por segundo.
- Ningún uso de modelos por esta función: no hay turnos nuevos salvo los que el dueño escribe.

## Avisos

El núcleo decide (`core/src/remote/push.rs`) y solo cuando el iPhone no está conectado a la sala:

| Aviso | Cuándo |
| --- | --- |
| Respuesta lista o fallida | El turno empezó desde el iPhone, o duró más de 2 minutos |
| Sesión de código | Pasa a `waiting`, `done` o `error` |
| Permiso pendiente | Siempre, marcado como urgente (el agente espera ~108 s) |
| Plan casi agotado, movimiento de Niko, tope de presupuesto, mensajitos, fin de concentración | Al emitirse el evento |

Cada tipo tiene su interruptor en el iPhone. Máximo 30 avisos por hora por máquina; los seguidos se agrupan por
máquina. El texto va cifrado y recortado a 300 caracteres. Las horas de silencio las pone el modo Concentración de
iOS.

## La app de iPhone

**Inicio.** La mascota grande y animada (los mismos sprites del núcleo, con el aspecto que el dueño le dio en esa
máquina), el selector «Mac / PC trabajo» con su estado, y debajo lo que importa ahora: respuestas en curso, sesiones
de código, permisos pendientes, uso de planes y los mensajitos del día. Tocar la mascota abre el chat.

**Chat.** Igual que en el escritorio: respuesta que se escribe en vivo, Markdown, tarjetas, fuentes, actividad del
agente, comandos «/», sugerencias, cola de pendientes. Micrófono en el compositor. Fotos desde la cámara o la galería.

**Historial.** Lista y búsqueda por máquina. Se guarda una copia en el iPhone para leerla sin conexión; con la
máquina apagada el compositor queda desactivado con «La Mac está apagada».

**Sesiones y permisos.** Lista de sesiones de Claude Code, Codex y Gemini con su estado; tarjeta de permiso con
título, resumen, detalle y los botones «Rechazar» y «Permitir» (Face ID).

**Voz.** Dictado en el dispositivo (SpeechAnalyzer, español), con el vocabulario propio de Buddy. El audio no sale
del iPhone; a la máquina solo viaja el texto. No hay palabra de activación en el iPhone: iOS no permite escuchar en
segundo plano.

**Face ID.** Al abrir la app y al volver tras un minuto fuera; para cada «Permitir»; para emparejar u olvidar una
máquina. Con el código del teléfono como respaldo.

**Accesos rápidos.** Un atajo «Hablar con Buddy» para el botón de Acción, Siri, el Centro de control y la pantalla
bloqueada: abre la app ya escuchando. «Compartir → Buddy» desde otras apps para texto, enlaces e imágenes.

**La mascota fuera de la app.**
- *Isla Dinámica y pantalla bloqueada (actividad en vivo):* aparece al empezar una respuesta, una sesión o un
  temporizador de concentración. Muestra la mascota en la pose del momento, la máquina y una línea de estado.
  Compacta: mascota a un lado, estado al otro. Expandida: además el agente y el progreso. Tocarla abre ese chat o
  esa sesión. Se cierra sola unos minutos después de terminar.
- *Qué viaja en esas actualizaciones:* iOS no deja descifrar las actualizaciones de una actividad en vivo, así que
  solo llevan códigos (pose, estado, número de máquina, agente, progreso), nunca texto de la conversación ni
  nombres de proyectos. Los nombres los pone el iPhone con lo que ya tiene guardado. Apple y el relé verían, como
  mucho, «pose 3, estado 2».
- *Widget de pantalla bloqueada y de inicio:* la mascota en reposo, qué máquinas están encendidas y el último
  mensajito. Tocarlo abre el chat.
- *Privacidad:* las notificaciones en la pantalla bloqueada dicen qué pasó («Buddy respondió», «Claude Code te
  espera») y no el texto de la conversación, hasta desbloquear. Un interruptor permite mostrar el texto. El widget
  oculta el mensajito con el teléfono bloqueado.
- Las poses se guardan como imágenes en el contenedor compartido de la app cuando se descargan los sprites, porque
  los widgets no pueden hablar con la máquina.

**Datos en el iPhone.** La copia del historial vive en SQLite con protección completa (ilegible con el teléfono
bloqueado), excluida de las copias de iCloud, y se borra al olvidar la máquina. La vista previa del selector de apps
se difumina. Sin analítica, sin bibliotecas de terceros con red, sin telemetría.

**Estética y accesibilidad.** Mismos tokens de `assets/design-tokens.json`. Texto dinámico, VoiceOver con etiquetas
en español en la mascota y los botones, objetivos táctiles de 44 pt, respeto de «Reducir movimiento» (la mascota
queda en su pose fija). Las pantallas se eligen con maquetas antes de construir la etapa 2.

**Código compartido con la Mac.** Lo que no dependa de AppKit ni de los tipos de UniFFI (tokens, pintor de píxeles,
Markdown) pasa a `apps/apple/Shared/` y lo usan las dos apps. Los registros JSON se decodifican con `Codable` y se
prueban contra ejemplos que generan las pruebas del núcleo, para que no se separen.

## Relé

- `POST /rooms` con la clave de dueño: crea la sala y devuelve su llave de acceso. Sin esa clave nadie crea salas.
- `GET /rooms/:id/ws` con la llave de la sala y el papel (`desktop` o `phone`): un socket por papel; reenvía cada
  trama al otro lado y notifica presencia.
- Trama especial del escritorio: «entrega este aviso» (normal o de actividad en vivo). El relé firma la petición a
  APNs con la clave `.p8`, que vive como secreto del Worker.
- Guarda por sala: hash de la llave, identificadores de avisos, última conexión. Nada de mensajes ni registros de
  contenido.
- Límites: tramas de hasta 256 KB (los adjuntos van en trozos), tope de tramas por minuto y de avisos por hora.
- `DELETE /rooms/:id` con la llave: borra todo lo de la sala.

## Errores

| Situación | Qué pasa |
| --- | --- |
| Relé inalcanzable | El escritorio reintenta con espera creciente; el iPhone muestra «Sin conexión» y el historial guardado. |
| Máquina dormida o apagada | El iPhone la muestra apagada; compositor desactivado. |
| QR caducado o ya usado | Se rechaza; el escritorio ofrece generar otro. |
| Fallo de cifrado o clave desconocida | Se corta la conexión, se anota y no se reintenta con esa clave. |
| Llamada fuera de la lista | Se rechaza y se anota en el registro de acciones. |
| Permiso que caduca antes de responder | `ApprovalClosed` retira la tarjeta; el agente pregunta en su terminal. |
| APNs rechaza el identificador | El relé lo borra; el iPhone lo registra de nuevo al abrir. |

## Pruebas

- `remote/`: vectores publicados de Noise KK; emparejado correcto, QR caducado, secreto equivocado, repetición;
  cifrado de avisos con contador y repetición.
- `core/src/remote/`: relé falso (un rasgo, como `Api` en Telegram). Lista blanca (cada llamada prohibida se
  rechaza), reenvío de eventos, agrupado de `ChatDelta`, reglas y tope de avisos, capturas negadas en turnos
  remotos, «Permitir» desactivado por el interruptor, registro de acciones.
- `relay/`: pruebas con el entorno local de Workers: sala, llave incorrecta, reenvío, presencia, límites, borrado.
- `apps/ios/`: decodificación de los ejemplos del núcleo, reglas de bloqueo con Face ID, estados de la actividad en
  vivo; el resto, en el simulador.

## Etapas

| Etapa | Entrega | Se acepta cuando |
| --- | --- | --- |
| 1. Núcleo y relé | `remote/`, `core/src/remote/`, `relay/` desplegado, Ajustes → iPhone en Mac y Windows, y un cliente de consola que hace de teléfono. No necesita la cuenta de Apple. | `cargo test` pasa; el cliente de consola se empareja con la Mac y con Windows, envía un mensaje y ve la respuesta en vivo; en el relé solo se ven bytes cifrados; en reposo, ~0 % de CPU. |
| 2. App de iPhone | Emparejado, máquinas, inicio con la mascota, chat, historial, sesiones, permisos con Face ID, voz, avisos. | Desde el iPhone con datos móviles: chat con la Mac y con la PC, aviso de respuesta lista con la app cerrada, permiso aprobado con Face ID, historial legible con la máquina apagada. |
| 3. La mascota fuera de la app | Actividad en vivo (Isla y pantalla bloqueada), widgets, botón de Acción, Siri, «Compartir → Buddy». | Una respuesta larga se sigue en la Isla sin abrir la app; tocar la mascota abre el chat; el botón de Acción abre Buddy escuchando. |
| 4. Auditoría | Revisión de seguridad de las tres plataformas y del relé con todo construido; correcciones. | Sin hallazgos altos ni críticos abiertos. |

Cada etapa tiene su plan de implementación antes de escribir código. Las API de iOS de la etapa 3 se confirman
contra el SDK 27 instalado al escribir su plan.

La auditoría de lo que ya existe (núcleo, Mac, Windows) está en
`docs/research/2026-10-03-auditoria-seguridad.md`; sus correcciones no esperan a la etapa 4. Cuatro de ellas son
condición de esta fase:

- La tarjeta de permiso muestra el comando entero antes de dejar pulsar «Permitir», y «Permitir siempre» deja de
  guardar reglas de solo programa (hallazgos A1 y M1). Sin esto no se construye `answer_approval` remoto.
- El núcleo valida `chat_id` antes de aceptar `send_message` desde el teléfono.
- El núcleo rechaza claves reservadas en `set_setting`, que además nunca entra en la lista de llamadas remotas.
- La app de iPhone no pide iconos de sitios por enlaces que vengan en el texto de una respuesta (hallazgo A3).

## Lo que hace el dueño

- Inscribirse en Apple Developer (antes de la etapa 2) y crear la clave de avisos `.p8`.
- Crear una cuenta gratuita de Cloudflare y desplegar el relé con un comando (antes de probar la etapa 1 fuera de
  su red).
- Pegar la dirección y la clave del relé en Ajustes → iPhone de cada máquina, y escanear el QR.

## Fuera de alcance

- Ejecutar modelos en el iPhone o en la nube.
- Fusionar los historiales de la Mac y la PC.
- Mensajes en cola para una máquina apagada; encender la máquina a distancia.
- Cambiar ajustes del escritorio desde el teléfono.
- Leer las respuestas en voz alta; palabra de activación en el iPhone.
- Apple Watch, iPad, Android y más de un iPhone por máquina.
- Publicar en la App Store (la app se instala desde Xcode con la cuenta del dueño).
