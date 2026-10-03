# Relé de Buddy

El punto de encuentro entre Buddy en tu Mac o Windows y Buddy en tu iPhone. Es un Cloudflare Worker con un Durable
Object por sala; cabe en el plan gratuito y lo despliegas en tu propia cuenta.

Todo lo que pasa por aquí va cifrado de extremo a extremo por las apps. El relé solo reenvía tramas opacas, avisa de
quién está conectado y entrega los avisos a Apple (APNs). El contrato exacto está en
`docs/phases/10-iphone-etapa-1-plan.md` («Contrato del relé»).

## Qué puede ver y qué no

**No puede ver:** tus mensajes, respuestas, archivos, nombres de chats o proyectos, ni las claves con las que se
cifran. Tampoco los guarda: una trama se reenvía y se olvida.

**Sí puede ver** (y, por tanto, también Cloudflare como dueño de la plataforma):

- Que existe una sala, cuándo se creó y cuándo se usó por última vez.
- Cuándo se conectan y desconectan el escritorio y el teléfono, desde qué dirección IP, y el tamaño y la hora de
  cada trama (nunca su contenido).
- Los identificadores de avisos del iPhone (los que da Apple), para poder entregar los avisos.
- Los códigos de la actividad en vivo (pose, estado, progreso: solo números), porque iOS no permite cifrarlos.

**Apple recibe**, por cada aviso, el texto fijo «Buddy · Novedad en tu equipo» y un bloque cifrado que solo tu iPhone
sabe abrir.

**Lo que guarda por sala:** el hash SHA-256 de la llave de la sala (nunca la llave), los identificadores de avisos,
la fecha de creación, la de última conexión y el contador de avisos de la hora en curso. Nada más. `DELETE` lo borra
todo.

**Registros:** solo el identificador de la sala y el tipo de evento («connect», «push alert 200»…). Los registros
persistentes de Cloudflare están desactivados; no los actives sin necesidad.

## Desplegar en tu cuenta

Necesitas Node 22 o posterior y una cuenta gratuita de Cloudflare (<https://dash.cloudflare.com/sign-up>).

1. Instala las dependencias:

   ```
   cd relay
   npm ci
   ```

2. Inicia sesión en Cloudflare (abre el navegador):

   ```
   npx wrangler login
   ```

3. Despliega. Al terminar muestra la dirección del relé, algo como `https://buddy-relay.<tu-subdominio>.workers.dev`:

   ```
   npx wrangler deploy
   ```

4. Crea la clave de dueño y guárdala como secreto. Es la que permite crear salas; mientras no exista, nadie puede:

   ```
   openssl rand -hex 32
   npx wrangler secret put OWNER_KEY
   ```

   Pega la clave generada cuando la pida.

5. Comprueba que responde (debe decir `buddy-relay`):

   ```
   curl https://buddy-relay.<tu-subdominio>.workers.dev/
   ```

6. En Buddy → Ajustes → iPhone escribe la dirección del relé y la clave de dueño. Buddy las guarda en el Llavero
   (Mac) o en el Administrador de credenciales (Windows).

### Avisos (más adelante, con la app del iPhone)

Hace falta una cuenta de desarrollador de Apple. En *Certificates, Identifiers & Profiles → Keys* crea una clave con
«Apple Push Notifications service (APNs)» y descarga el archivo `AuthKey_XXXXXXXXXX.p8` (solo se descarga una vez).

```
npx wrangler secret put APNS_KEY_P8 < AuthKey_XXXXXXXXXX.p8
npx wrangler secret put APNS_KEY_ID     # el identificador de la clave (10 caracteres)
npx wrangler secret put APNS_TEAM_ID    # el identificador de tu equipo de Apple
npx wrangler secret put APNS_TOPIC      # el identificador (bundle id) de la app del iPhone
```

Mientras falte alguno de los cuatro, el relé no llama a Apple y responde `{"t":"error","code":"no-apns"}` a cada
aviso. El resto funciona igual.

Los secretos viven solo en Cloudflare. No los escribas en `wrangler.jsonc` ni los subas al repositorio.

## Límites

| Límite | Qué pasa al superarlo |
| --- | --- |
| Tramas de hasta 256 KB | `{"t":"error","code":"too-big"}` y cierre con código 1009 |
| 600 tramas por minuto por socket | `{"t":"error","code":"rate"}`; la trama se descarta |
| 60 avisos por hora por sala | `{"t":"error","code":"rate"}`; el aviso se descarta |
| Un socket por papel | El socket anterior se cierra con código 4001 |

Al borrar la sala sus sockets se cierran con código 4004.

## Desarrollo

```
npm test            # pruebas (vitest dentro del entorno de Workers)
npm run typecheck   # tsc --noEmit
npm run dev         # relé local en http://localhost:8787
```

Para el relé local, crea `relay/.dev.vars` (no se sube al repositorio) con `OWNER_KEY=una-clave-de-prueba`.

| Archivo | Qué hace |
| --- | --- |
| `src/index.ts` | Rutas HTTP y clave de dueño |
| `src/room.ts` | La sala (Durable Object): sockets, reenvío, presencia, límites |
| `src/apns.ts` | Firma del token de proveedor y envío a APNs |
