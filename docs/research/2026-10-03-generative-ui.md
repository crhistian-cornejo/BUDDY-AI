# Generative UI dentro del chat de Barry

Investigación: 3 de octubre de 2026. Revisión de documentación oficial y del checkout actual de Buddy. Propuesta de arquitectura; no se han instalado dependencias ni implementado cambios de producto. «Sonic» se interpreta provisionalmente como Sonnet, que sí aparece en el catálogo local.

## Decisión propuesta

Incorporar artefactos visuales como partes tipadas de una respuesta. Mantener los proveedores y la orquestación Rust actuales. Empezar con un catálogo reducido de componentes y gráficos declarativos, usando un renderer web compartido para Mac y Windows. Después incorporar vistas de código aisladas y soporte para MCP Apps externos.

El modelo elige componentes y genera sus datos/configuración; la aplicación valida, dibuja y gestiona las interacciones. Gemini no es un requisito del renderer. Haiku, Gemini Flash y Sonnet pueden usar las mismas herramientas, aunque su fiabilidad debe medirse.

## Cómo lo hacen otros productos

| Producto | Comportamiento público documentado | Qué tomar para Buddy |
| --- | --- | --- |
| Claude | Visualizaciones interactivas dentro de la respuesta, modificables durante la conversación. Se distinguen de artifacts permanentes en un panel. | Visual pequeño inline; dashboard ampliable y persistente. |
| ChatGPT | Herramientas MCP con recursos UI que se ejecutan en un iframe y usan un puente de mensajes. | Separar herramientas de datos de herramientas de presentación. |
| Claude Desktop y otros hosts MCP Apps | Recursos HTML interactivos asociados a herramientas, con comunicación bidireccional. | Implementar soporte de host si Buddy debe mostrar apps MCP externas. |
| LibreChat | React, HTML, SVG, Markdown y Mermaid como artifacts; usa Sandpack para HTML/JS. Compatible con los modelos disponibles del agente. | Referencia abierta para previews, revisiones y ampliación de artifacts. |

Fuentes: [Claude: visualizaciones inline](https://claude.com/blog/claude-builds-visuals), [ChatGPT: UI MCP](https://developers.openai.com/plugins/build/chatgpt-ui), [MCP Apps](https://modelcontextprotocol.io/extensions/apps/overview), [LibreChat artifacts](https://www.librechat.ai/docs/features/artifacts).

Estos documentos describen comportamiento público y APIs. No establecen cómo funcionan internamente todas las visualizaciones propias de ChatGPT o Claude. Tener acceso al modelo mediante una CLI no transfiere automáticamente el renderer de esos productos.

## Librerías y estándares

| Opción | Papel | Encaje propuesto |
| --- | --- | --- |
| json-render | Catálogo tipado de componentes y acciones; el modelo genera una especificación JSON; estado, bindings y streaming. | Primera candidata para dashboards y controles. Se puede empaquetar un renderer React pequeño sin migrar todo el chat. |
| A2UI | Protocolo declarativo de UI; el cliente mapea componentes a su framework y conserva control de estilo. | Alternativa si se prioriza renderizado nativo en SwiftUI y web. Comprobar implementación y cobertura concretas de cada renderer antes de adoptarlo. |
| MCP Apps | Recursos UI HTML asociados a herramientas MCP y protocolo de comunicación con el host. | Interoperabilidad con herramientas visuales externas. No sustituye al motor que genera el contenido. |
| AI SDK UI | Streaming de mensajes y representación de resultados de herramientas como componentes. | Referencia útil; adopción opcional. Buddy ya tiene transporte y proveedores Rust/CLI. |
| CopilotKit / AG-UI | Primitivas de UI generativa y sincronización de estado entre agentes y frontend. | Más atractivo si se reconstruye una experiencia React amplia; no necesario para el primer renderer. AG-UI y A2UI cumplen funciones distintas. |
| Tambo | Selección de componentes React registrados y generación de props según un esquema Zod. | Alternativa más integrada; revisar alojamiento, proveedores y adaptación al backend actual. |
| Vega-Lite | Gráficos interactivos declarativos mediante JSON. | Componente Chart del catálogo; evita que el modelo escriba código de gráficos cada vez. |

Fuentes: [json-render](https://json-render.dev/docs), [A2UI](https://a2ui.org/introduction/what-is-a2ui/), [AI SDK UI](https://ai-sdk.dev/docs/ai-sdk-ui/generative-user-interfaces), [CopilotKit](https://docs.copilotkit.ai/concepts/generative-ui-overview), [Tambo](https://docs.tambo.co/concepts/generative-interfaces/generative-components), [Vega-Lite](https://vega.github.io/vega-lite/).

No confundir el renderer web integrado en WKWebView con controles nativos SwiftUI. Compartir el renderer web reduce duplicación; un renderer SwiftUI propio añade trabajo por componente. Para una integración nueva de AI SDK, la documentación recomienda AI SDK UI frente a AI SDK RSC experimental.

## Skills

json-render ofrece skills de core, React, shadcn, MCP y otros renderers: [documentación de skills](https://json-render.dev/docs/skills). Son útiles para el agente que implementa la integración. No habilitan por sí solas una superficie visual en Buddy.

Crear después una skill propia `visualizar` para Barry: cuándo usar cada herramienta, ejemplos pequeños del catálogo, cómo referenciar datasets, cómo actualizar un artifact existente y cómo tratar datos ausentes. Cargarla con `use_skill`, que ya existe, para evitar añadir toda la guía a cada turno.

La skill debe enseñar el contrato real de Buddy, no instrucciones genéricas para producir JSX que el chat no puede ejecutar. Catálogo y validación viven en código; la skill explica su uso.

## Estado del repositorio observado

- `core/src/router.rs`: catálogo con Haiku, Sonnet y Gemini 3.8 Flash, entre otros. Ya admite selección de esfuerzo.
- `core/src/providers/gemini.rs`: integración con `agy`, MCP y modelos con sufijos de esfuerzo. Para la CLI, Flash con esfuerzo bajo se traduce a `gemini-3.8-flash-low`; ese sufijo no debe trasladarse como ID a la API de Google.
- `core/src/providers/mod.rs`: `TurnEvent` lleva texto, actividad de herramientas, fuentes, uso y finalización; no hay una variante de artifact visual.
- `core/src/events.rs`: eventos de texto, herramientas y fuentes para las apps; falta el evento visual tipado.
- `core/src/store.rs`: mensajes con texto y adjuntos; hace falta persistir tipo, versión, datos y estado de los artifacts.
- `hook/src/mcp.rs`: servidor MCP propio; candidato para exponer herramientas visuales.
- `core/src/skills.rs`: skills con carga progresiva; no otorgan capacidades adicionales.
- `apps/macos/Sources/Chat/ChatViews.swift`: respuesta SwiftUI y tarjetas de archivos; falta una vista visual integrada.
- `apps/windows/src/chat/answer.ts`: renderer DOM de Markdown. Un enlace Markdown de imagen se representa actualmente como texto con icono, no como imagen generada visible.
- `apps/windows/package.json`: TypeScript/Vite/Tauri, sin React instalado.

Se observaron modificaciones locales preexistentes. La propuesta no las modifica ni asume que estén validadas.

## Contrato mínimo propuesto

Herramientas orientativas, todavía inexistentes:

- `render_chart(dataset_id, chart_spec)` para gráficos.
- `render_dashboard(dataset_ids, ui_spec)` para composición de métricas, tablas y filtros.
- `render_interactive_view(source)` para una vista libre, en fase posterior.
- `generate_image(prompt, reference_asset_ids)` para imágenes raster.
- `update_visual(artifact_id, expected_revision, changes)` para revisiones posteriores.

Flujo: solicitud → Barry/modelo → herramienta → validación Rust → persistencia de artifact → evento a las apps → renderer inline. La herramienta debe publicar el artifact mediante el núcleo/relay, no depender de que cada parser de CLI preserve HTML o metadata UI.

En almacenamiento, relacionar artifacts con chat, mensaje y llamada de herramienta. Campos: ID, tipo, versión de esquema, revisión, título, resumen accesible, referencias a datos/assets y estado de presentación. No reutilizar adjuntos como único contrato: no expresan acciones ni revisiones.

Para el primer catálogo bastan Stack/Grid, Card, Metric, Chart, Table, Select, Tabs y Slider. Las dependencias gráficas se empaquetan con la app. El modelo puede componer esos elementos, sin importar paquetes ni ejecutar JavaScript arbitrario.

Mac: WKWebView dedicado al área del artifact, incrustado en la respuesta SwiftUI. Windows: renderer web equivalente dentro de una superficie aislada. Se comparten catálogo, assets y renderer. Ajustar altura, tema, accesibilidad y botón Ampliar.

La vista del artifact no recibe el puente Tauri general ni acceso a archivos/comandos. Exponer solo acciones tipadas y validadas por el núcleo. Para código generado, usar una superficie separada con política de contenido y red restringidas; una WKWebView por sí sola no constituye una política de aislamiento completa.

Filtros, hover, pestañas y sliders trabajan localmente cuando sea posible. Solo nuevas consultas o cambios semánticos requieren otro turno del modelo. Las acciones que escriben datos externos pasan por las reglas de permisos existentes.

## Modelos y generación de imágenes

Google documenta `gemini-3.8-flash` con function calling, structured outputs y thinking low/medium/high. No admite `minimal`; su salida es texto y no soporta generación de imágenes: [modelo](https://ai.google.dev/gemini-api/docs/models/gemini-3.8-flash).

Por eso un dashboard y una ilustración tienen rutas distintas. El dashboard puede generarse con Flash bajo, Haiku o Sonnet componiendo JSON; la ilustración requiere una herramienta conectada a un modelo de imágenes, por ejemplo Gemini 3.1 Flash Image o Gemini 3 Pro Image: [generación de imágenes](https://ai.google.dev/gemini-api/docs/image-generation).

No se ha verificado que la integración `agy` de Buddy exponga esa generación de imágenes. La disponibilidad mediante una suscripción tampoco establece acceso o facturación por API. Verificar el proveedor elegido antes de implementarla; un renderer local no añade consumo de inferencia, la generación y las revisiones sí.

Propuesta de routing: modelo actual para decidir y producir specs sencillas; Gemini Flash bajo como opción cuando se solicite; escalar esfuerzo o modelo ante errores de validación o interacciones complejas. No sustituir automáticamente un Haiku/Sonnet elegido por el usuario. Los resultados de calidad y latencia todavía son hipótesis: no se han ejecutado benchmarks.

## Implementación por etapas

1. **Primer incremento:** artifacts tipados y persistentes, renderer de imágenes y un Chart declarativo en ambas apps. Mostrar y ampliar un gráfico, modificarlo en el turno siguiente y reabrirlo desde historial.
2. **Dashboards:** catálogo de componentes, filtros locales, tablas y revisiones. Añadir la skill `visualizar` cuando las herramientas existan.
3. **Vistas libres:** HTML/JS aislado para simulaciones, juegos o diseños fuera del catálogo; errores visibles y opción de reparación.
4. **Interoperabilidad:** host MCP Apps para herramientas externas; negociación de capacidad, recursos `ui://`, puente y política de permisos.

Validación necesaria: datasets conocidos con valores verificables; specs inválidas; referencias inexistentes; historial/revisiones; comportamiento equivalente Mac/Windows; aislamiento del puente nativo. Evaluar el mismo conjunto de peticiones con Haiku, Flash bajo y Sonnet bajo: éxito de render, exactitud de datos, latencia, tokens y reparaciones. No prometer que todos producen igual calidad.

Ejemplo de aceptación: «Muéstrame mis gastos por categoría» produce barras con datos reales; «solo septiembre» actualiza el artifact; el selector cambia categorías sin nueva llamada al modelo; cerrar y abrir el chat conserva la visualización. «Hazme una imagen de Barry» utiliza el generador de imágenes y muestra el archivo inline.
