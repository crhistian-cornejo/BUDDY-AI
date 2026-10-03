---
name: hojas
description: Crear una hoja de Excel útil (presupuesto, seguimiento, comparativa, tabla de datos con totales) cuando el usuario pide un Excel o una tabla para trabajar.
---
# Hojas de cálculo

1. Primera fila: encabezados claros; una fila por elemento, una columna por dato.
2. Usa fórmulas de verdad para totales, promedios y porcentajes (`=SUM(B2:B20)`, `=B2/C2`), nunca números ya
   calculados a mano.
3. Da formato por columna: moneda, porcentaje o fecha; activa el autofiltro en tablas largas.
4. Si hay varias vistas (datos, resumen), usa varias hojas.
5. Si el usuario dio un Excel, léelo antes con `read_document`.
6. Crea el archivo con `create_spreadsheet` y responde en dos líneas qué hace y dónde quedó.
