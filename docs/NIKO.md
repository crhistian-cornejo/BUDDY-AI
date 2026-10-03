# Niko · Finanzas

Niko registra gastos, pagos, ingresos, transferencias y vouchers en Notion. Lee Gmail y Drive; no envía, borra ni etiqueta correos, ni mueve dinero. La lógica compartida vive en `core/src/niko.rs`; Mac y Windows ofrecen Ajustes › Agentes › Niko.

## Activarlo

1. Conecta Gmail, Google Drive y Notion en claude.ai › Ajustes › Conectores.
2. Abre Claude Code y usa `/mcp` para autorizar los conectores que indiquen «Needs authentication». Que claude.ai diga conectado no garantiza que Claude Code tenga el acceso vigente.
3. En Ajustes › Agentes › Niko, pulsa «Volver a comprobar». Comparte con el conector la página de Notion donde guardarás tus finanzas y pega su enlace.
4. Pulsa «Revisar ahora». La primera revisión prepara Movimientos, Presupuestos y Dashboard dentro de esa página. Usa Sonnet para la preparación y Haiku para las revisiones siguientes.
5. Activa «Niko revisa el correo en este equipo» en un solo equipo. Puedes elegir 10, 20, 30 o 60 minutos. El dashboard vive en Notion y se puede consultar desde la web, Mac o Windows.

La revisión automática está apagada inicialmente. Las credenciales de estos conectores las administra Claude; Buddy no las guarda. Los errores de autenticación y límites del proveedor necesitan resolverse antes de comprobar una revisión real.

## Estado verificado

La implementación, las interfaces y las pruebas están completas. La compilación de Mac, la compilación web de Windows y las pruebas locales pasan. En la comprobación realizada al terminar esta integración, Claude Code informó Gmail y Drive conectados, y Notion pendiente de autorización. No se creó ni se verificó un dashboard real durante esta revisión.
