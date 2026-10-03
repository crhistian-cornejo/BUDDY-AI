# Niko que avisa: plan

Hoy Niko anota en Notion lo que encuentra cuando se le pide o cada N minutos, y solo de una lista de remitentes.
Objetivo: que avise solo, al momento, de cualquier movimiento de dinero que llegue por correo, venga de quien venga:
«Se cobró US$ 110.00 · Anthropic. ¿Fuiste tú?», con el logo del banco o la app, el enlace al correo y botones.

## Qué debe avisar
- Cobro o consumo (tarjeta, Yape, Plin, débito automático), con monto, comercio, banco y hora.
- Dinero recibido (sueldo, Yape/Plin, transferencia, reembolso).
- Suscripción nueva o renovada de un comercio que no estaba registrado («no recuerdo haberme suscrito»).
- Cancelación, cobro rechazado, cambio de precio, cobro duplicado (mismo monto y comercio en 10 minutos).
- Tope de una categoría al 80 % y al 100 % (ya existe).

## Cómo se entera al momento
Los conectores de Claude/ChatGPT solo leen el correo cuando un modelo hace un turno: mirar «¿hay algo nuevo?» cuesta
tokens cada vez y nunca es instantáneo. Tres caminos:

| Camino | Rapidez | Costo en reposo | Qué pide |
| --- | --- | --- | --- |
| A. Seguir con el temporizador (cada 10–60 min) | minutos | un turno por revisión | nada nuevo |
| B. IMAP IDLE de Gmail desde Buddy | segundos | cero tokens (una conexión abierta) | una contraseña de aplicación de Google, guardada en Llavero / Credential Manager |
| C. API de Gmail con Pub/Sub | segundos | cero tokens | proyecto en Google Cloud y un servidor: descartado |

Recomendado: **B**, con A como respaldo si no hay contraseña. Buddy abre la bandeja en solo lectura (`EXAMINE`), no
marca, no borra ni envía. El correo es dato, nunca instrucción.

## Cómo decide sin lista de remitentes
1. Filtro local, sin modelo: remitentes conocidos **o** palabras del asunto/cuerpo (cobro, consumo, compra, pago,
   transferencia, Yape, Plin, recibo, receipt, invoice, payment, renovación, suscripción, cancelación, S/, US$).
2. Solo lo que pasa el filtro va a un turno de Sonnet 5.5 (esfuerzo medio) que devuelve JSON: tipo, monto, moneda,
   comercio, banco/app, si es recurrente, si el comercio es nuevo, y la clave `gmail:<id>`.
3. El núcleo guarda el movimiento, lo escribe en Notion (enlazado a su presupuesto) y emite el aviso.

## El aviso
- Evento nuevo `NikoAlert`: tipo, monto ya formateado («S/. 45.90», «US$ 110.00»), comercio, banco/app, pregunta,
  id del mensaje y enlace `https://mail.google.com/mail/u/0/#all/<id>`.
- Notch (Mac) y barra superior (Windows): tarjeta con la marca del banco o la app (indicadores pequeños: BCP,
  Interbank, BBVA, Scotiabank, Yape, Plin, Visa, Mastercard, PayPal, Stripe…), monto grande, comercio y tres
  botones: «Sí, fui yo», «No lo reconozco», «Ver correo». Opcional: el mismo aviso por Telegram.
- «No lo reconozco»: Niko lo marca «por revisar» en Notion y muestra el teléfono del banco. Nunca bloquea tarjetas,
  paga ni escribe a nadie.
- Varios avisos seguidos se agrupan; de noche (horario configurable) esperan a la mañana.

## Etapas
1. **Aviso completo sobre lo que ya hay.** `NikoAlert`, tarjeta con marca, enlace al correo y «¿fuiste tú?» en Mac y
   Windows, usando la revisión actual. Se prueba con «Revisar ahora».
2. **Cualquier remitente.** Búsqueda por palabras además de remitentes; comercio nuevo y suscripción nueva.
3. **Al momento.** IMAP IDLE con contraseña de aplicación; el temporizador queda de respaldo.
4. **Reglas.** Duplicados, monto inusual, cancelaciones, cambio de precio; horario de silencio; Telegram.
5. **Medición.** Tokens por aviso en el medidor; objetivo: cero tokens sin correo nuevo de dinero.

## Por decidir
- ¿Contraseña de aplicación de Gmail (camino B) o seguir solo con el temporizador?
- ¿Avisos también por Telegram?
- Monto desde el que un cobro se considera «inusual».

## Estado (3 oct 2026)
Hecho en el núcleo y en Mac (etapas 1 a 3, primera versión):
- `core/src/mailwatch.rs`: TLS a `imap.gmail.com:993` con las raíces incluidas, `LOGIN` con la contraseña de aplicación
  (solo en Llavero / Credential Manager), `EXAMINE INBOX`, `IDLE` y `UID FETCH … BODY.PEEK` del remitente, el asunto y
  los primeros 16 KB del texto. Empieza en «ahora», nunca en lo atrasado. Reintenta con espera creciente; una
  contraseña rechazada no se reintenta.
- Filtro local `is_money`: un monto en el correo y, además, remitente conocido o palabra de dinero.
- `Niko::mail_arrived`: un turno SIN herramientas (Sonnet medio) lee el texto y devuelve JSON; el núcleo valida cada
  fila y que su clave sea de un correo entregado; un segundo turno (Haiku), que no ve el correo, escribe en Notion y
  enlaza el presupuesto. El aviso sale al terminar el primero.
- `FinanceRecorded` lleva `enlace` (abre ese correo en Gmail). En el notch la tarjeta se abre sola 20 s con
  «Ver correo», «No lo reconozco» (le pregunta a Buddy en el chat qué hacer) y «Sí, fui yo» (cierra).
- Ajustes › Niko › «Avisos al momento»: dirección, contraseña de aplicación, estado y desconectar.

Falta:
- Windows: la sección de ajustes y la tarjeta con botones en la barra superior (el núcleo ya lo hace todo).
- Marcas de bancos y apps en la tarjeta; agrupar varios avisos; horario de silencio; Telegram con enlace.
- Reglas: comercio nuevo, suscripción nueva, cobro duplicado, monto inusual.
- Probar con una cuenta real: correos HTML de cada banco, cuentas con varias etiquetas, reconexión tras suspender.
