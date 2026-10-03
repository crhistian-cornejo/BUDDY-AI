# Buddy en el iPhone · Etapa 1 (núcleo y relé): plan de implementación

> **Para quien lo ejecute:** se implementa tarea por tarea, con la prueba antes del código. Los pasos usan casillas.

**Objetivo:** que un cliente de consola que hace de teléfono se empareje con Buddy en Mac o Windows a través de un
relé, chatee por un canal cifrado de extremo a extremo y reciba sus eventos.

**Arquitectura:** un crate nuevo sin entrada ni salida (`remote/`) con el emparejado, el canal Noise y el cifrado de
avisos; un módulo en el núcleo (`core/src/remote/`) con la sesión, la lista blanca de llamadas, el reenvío de eventos
y las reglas de avisos; un relé (`relay/`, Cloudflare Worker) que solo reenvía tramas opacas.

**Tecnología:** Rust (`snow`, `chacha20poly1305`, `hkdf`, `hmac`, `sha2`, `x25519-dalek`, `tokio-tungstenite`),
TypeScript (Workers, Durable Objects, vitest).

**Especificación:** `docs/phases/10-iphone.md`.

## Restricciones globales

- Secretos solo en Llavero / Credential Manager; nunca en SQLite, registros ni eventos.
- Solo conexiones salientes; sin puertos abiertos.
- En reposo ~0 % de CPU: sin iPhone emparejado no existe conexión; con uno, lectura bloqueada sin sondeos.
- Reintentos 2 s, 4 s, 8 s… hasta 5 min.
- La lógica vive en `core`; las apps solo dibujan. Todo texto de interfaz en español.
- El relé no guarda mensajes. Tramas de hasta 256 KB.
- `ChatDelta` agrupado a un máximo de 5 envíos por segundo; sin iPhone conectado no se envía ningún evento.
- Avisos: máximo 30 por hora por máquina; texto recortado a 300 caracteres.
- QR válido 5 minutos y de un solo uso; cinco intentos fallidos lo anulan.
- Código portado de MIKA conserva su aviso MIT. Nada de `reuse/mika/` se edita.

## Foco de revisión

1. **El relé reenvía una trama `pair` falsa o repetida** → se rechaza sin gastar el QR legítimo más de cinco veces.
2. **Llega una llamada fuera de la lista (o `set_setting`)** → error «llamada no permitida» y entrada en el registro.
3. **`chat_id` con `..` o separadores en `send_message` / `messages`** → error, sin tocar el disco.
4. **El teléfono se desconecta a mitad de una respuesta** → dejan de enviarse eventos y, al terminar, sale un aviso.
5. **Una trama cifrada repetida o fuera de orden** → la sesión se corta y exige un nuevo saludo; nunca se procesa.

## Contrato del relé (lo usan las tareas 3, 5 y 6)

HTTP, todo con `Authorization: Bearer …`:

| Petición | Clave | Respuesta |
| --- | --- | --- |
| `POST /rooms` | clave de dueño | `201 {"room":"<32 hex>","key":"<64 hex>"}` |
| `GET /rooms/<room>/ws?role=desktop\|phone` | llave de la sala | `101`; `401` llave incorrecta; `404` sala desconocida |
| `DELETE /rooms/<room>` | llave de la sala | `204`; cierra los sockets y borra todo |
| `GET /` | — | `200 buddy-relay` |

Un segundo socket con el mismo papel sustituye al primero (cierre `4001`). Tramas de texto JSON con campo `t`:

| `t` | De → a | Qué hace el relé |
| --- | --- | --- |
| `pair`, `paired`, `hs1`, `hs2`, `msg` | cliente → el otro papel | Reenvía tal cual; si el otro no está, responde `{"t":"peer","on":false}` |
| `peer` (`on`: bool) | relé → cliente | Al conectar y cada vez que cambia la presencia del otro |
| `token` (`alert`, `live`, `sandbox`) | teléfono → relé | Guarda los identificadores de avisos; no reenvía |
| `push` (`kind`: `alert`\|`live`, `d`, `urgent`, `collapse`) | escritorio → relé | Entrega a APNs; sin claves de Apple responde `{"t":"error","code":"no-apns"}` |
| `error` (`code`) | relé → cliente | `too-big`, `rate`, `no-apns`, `bad-frame` |

## Estructura de archivos

| Archivo | Responsabilidad |
| --- | --- |
| `remote/src/pairing.rs` | Oferta del QR, saludo del teléfono, verificación con HMAC, caducidad |
| `remote/src/channel.rs` | Canal Noise KK: saludo y tramas cifradas, troceado de mensajes largos |
| `remote/src/push.rs` | Clave de avisos (HKDF) y sellado XChaCha20-Poly1305 con contador |
| `remote/src/wire.rs` | Sobres del relé (`t`, `d`) y tramas internas (`call`, `reply`, `event`) |
| `core/src/remote/session.rs` | Máquina de estados sin red: emparejado, saludo, llamadas, eventos |
| `core/src/remote/rpc.rs` | Lista blanca de llamadas y validación de argumentos |
| `core/src/remote/push.rs` | Qué evento merece aviso; tope por hora |
| `core/src/remote/link.rs` | Transporte: WebSocket saliente, reintentos, creación y borrado de sala |
| `core/src/remote/mod.rs` | Servicio `Remote`: estado, claves, emparejar, olvidar, registro de acciones |
| `relay/src/index.ts`, `relay/src/room.ts`, `relay/src/apns.ts` | Rutas, sala (Durable Object), envío a APNs |
| `remote/examples/telefono.rs` | Cliente de consola que hace de teléfono |
| `apps/macos/Sources/Settings/PhoneSection.swift`, `apps/windows/src/settings/…` | Ajustes → iPhone |

## Tareas

### Tarea 1: emparejado (`remote/src/pairing.rs`)

**Produce:** `Offer::new(relay, room, room_key, desktop_public, now) -> Offer`; `Offer::uri() -> String`
(`buddy://pair?d=<base64url(json)>`); `Offer::parse(uri) -> Result<Offer>`; `hello(offer, phone_public, name) -> Hello`;
`Offer::verify(&Hello, now) -> Result<Peer, PairError>`; `ack(offer, &Peer) -> String`; `check_ack(offer, …) -> bool`.

- [ ] Pruebas: ida y vuelta del URI; saludo correcto verifica; secreto equivocado → `BadProof`; caducado → `Expired`;
      nombre con saltos de línea queda en una línea de 60 caracteres; URI truncado o de otra versión → error.
- [ ] Implementar; `cargo test -p buddy-remote pairing`.

### Tarea 2: canal y avisos (`remote/src/channel.rs`, `remote/src/push.rs`)

**Produce:** `Keys::generate() -> Keys {private, public}`; `Channel::initiator(&Keys, their_public, room) -> (Handshake, hs1)`;
`Channel::responder(&Keys, their_public, room, hs1) -> Result<(Channel, hs2)>`; `Handshake::finish(hs2) -> Result<Channel>`;
`Channel::seal(&[u8]) -> Vec<Vec<u8>>`; `Channel::open(&[u8]) -> Result<Option<Vec<u8>>>` (None: faltan trozos);
`push::key(&Keys, their_public, secret, room) -> [u8; 32]`; `push::seal(key, counter, now, &Value) -> Vec<u8>`;
`push::open(key, bytes, last_counter, now) -> Result<(u64, Value)>`.

- [ ] Pruebas: saludo completo y mensaje en ambos sentidos; mensaje de 300 KB troceado y recompuesto; clave fija
      equivocada → el saludo falla; trama repetida o alterada → error; ambas partes derivan la misma clave de avisos;
      aviso con contador repetido o de hace más de 48 h → error.
- [ ] Implementar; `cargo test -p buddy-remote`.

### Tarea 3: sobres y tramas (`remote/src/wire.rs`)

**Produce:** `Envelope { t, d }` con `Envelope::parse(&str)` / `to_text()`; `Frame::{Call{id,call,args}, Reply{id,ok,err}, Event{event}}`.

- [ ] Pruebas: forma JSON exacta de cada sobre del contrato; trama desconocida → error; sobre de más de 256 KB → error.

### Tarea 4: sesión, llamadas y avisos (`core/src/remote/{session,rpc,push}.rs`)

**Consume:** tareas 1–3. **Produce:** `Session::new(host, keys, peer, opts)`, `Session::on_frame(&str, now) -> Vec<Out>`,
`Session::on_event(&Event, now) -> Vec<Out>`, `Out::{Send(String), Log(String)}`; rasgo `Host` con una función por
llamada de la lista blanca; `rpc::dispatch(&dyn Host, call, args, opts) -> Result<Value, String>`;
`push::decide(&Event, &Context) -> Option<Notice>`.

- [ ] Pruebas con un teléfono simulado (usa `buddy-remote`) y un `Host` falso: `hello` y `chats` responden;
      `set_setting` y cualquier llamada desconocida se rechazan y se anotan; `chat_id` con `..`, `/` o `\` se rechaza;
      `answer_approval(allow)` se rechaza con el interruptor apagado y «rechazar» siempre pasa; `ScreenshotRequest`,
      `SettingChanged`, `MediaCommand` no se reenvían; diez `ChatDelta` en 100 ms salen como una trama; sin teléfono
      conectado `on_event` no envía eventos y sí un aviso para `ApprovalRequest`; el aviso 31 de la hora no sale;
      trama cifrada repetida corta la sesión; cinco `pair` falsos anulan la oferta.
- [ ] Implementar; `cargo test -p buddy-core remote`.

### Tarea 5: transporte y servicio (`core/src/remote/{link,mod}.rs`, `core/src/lib.rs`, `core/src/store.rs`)

**Produce (API del núcleo, también por UniFFI y Tauri):** `remote_status() -> RemoteStatus`,
`remote_set_relay(url, owner_key)`, `remote_pair() -> PairOffer {uri, qr_size, qr_cells, expires_at}`, `remote_forget()`,
`remote_set_approvals(bool)`, `remote_log() -> Vec<RemoteAction>`; evento `RemoteChanged`.

- [ ] Pruebas con un relé en proceso (servidor WebSocket local en la prueba): crear sala, emparejar, saludo, `hello`
      de extremo a extremo; el relé cae → reintento con espera; `remote_forget` borra claves y pide borrar la sala;
      `set_setting` rechaza las claves `remote.*`, `folders.*`, `agent.*.permisos`, `commands.enabled`, `telegram.chat`.
- [ ] Implementar; `cargo test --workspace`.

### Tarea 6: relé (`relay/`)

- [ ] Pruebas (vitest con el entorno de Workers): crear sala sin clave de dueño → 401; llave incorrecta → 401; sala
      desconocida → 404; reenvío entre papeles; presencia al conectar y desconectar; sustitución de socket (4001);
      trama de más de 256 KB → `too-big`; tope de tramas → `rate`; `push` sin claves → `no-apns`; `push` con claves
      llama a APNs con el cuerpo esperado (fetch simulado); `DELETE` borra y cierra.
- [ ] Implementar; `npm test` en `relay/`. `relay/README.md` con el despliegue (`npx wrangler deploy`, secretos).

### Tarea 7: cliente de consola (`remote/examples/telefono.rs`)

- [ ] `cargo run -p buddy-remote --example telefono -- '<buddy://pair?...>'`: se empareja, saluda, llama a `hello` y
      `chats`, envía un mensaje e imprime los eventos. Se usa en la aceptación de la etapa.

### Tarea 8: Ajustes → iPhone (Mac y Windows)

- [ ] Sección con: dirección y clave del relé, «Emparejar» (QR pintado desde `qr_cells`), estado, «Olvidar iPhone»,
      interruptor «Aprobar permisos desde el iPhone», registro de acciones. Mismo aspecto en las dos.

## Aceptación de la etapa

`cargo test --workspace` pasa; el cliente de consola se empareja con la Mac y con Windows, envía un mensaje y ve la
respuesta en vivo; en el relé solo se ven bytes cifrados; en reposo, ~0 % de CPU.
