# Fase 7: voz

## Decisión (3 oct 2026)
Se probó la frase de activación «Hey Buddy» y se quitó. Siri la detecta con un modelo diminuto en un coprocesador de
bajo consumo que no está abierto a apps de terceros; sin eso, el transcriptor trabaja con cualquier sonido de la sala
(se midió ~10 % de CPU en Buddy y ~8 % en `corespeechd` con un video sonando), lo que rompe la regla de reposo ≈ 0 %.
En su lugar: dictado en el compositor, solo mientras el botón del micrófono está encendido.

## Hecho en Mac (macOS 27)
- Botón de micrófono en el compositor (`Dictation`, `apps/macos/Sources/Voice`): lo dicho se escribe en el borrador,
  detrás de lo ya tecleado; otro clic o enviar lo detiene. Un halo sigue el nivel de la voz.
- `CaptureInputSequenceProvider` (el sistema captura y convierte el micrófono) → `SpeechAnalyzer` +
  `SpeechTranscriber`, en español si el Mac lo tiene y, si no, en el idioma del sistema. El audio no sale del equipo.
- Permisos de micrófono y reconocimiento de voz; los errores se muestran sobre el campo.
- `SpeechDetector` no se usa: con captura en vivo termina la sesión con «RecogRejected» en macOS 27.0.

## Pendiente
- Windows: el mismo botón, con captura WASAPI y whisper.cpp local.
- Probar con voz real y ruido de fondo; elegir micrófono si hay varios.
- Mac: Atajos/Siri («Pregúntale a Buddy…») como forma de llamarlo por voz sin micrófono propio siempre abierto.
