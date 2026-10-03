# apps/macos

App de Mac en SwiftUI/AppKit sobre `core` (UniFFI): mascota, notch, chat, voz (SpeechAnalyzer), Siri/Atajos. Proyecto con XcodeGen (`project.yml`; el `.xcodeproj` no se sube).

```bash
xcodegen
xcodebuild -project Buddy.xcodeproj -scheme Buddy -configuration Debug build
```

La fase «buddy-core (Rust + UniFFI)» del build ejecuta `scripts/build-core-macos.sh`: compila el núcleo como biblioteca estática y genera `Generated/` (enlaces Swift, ignorado en git). Necesita `cargo`.

El notch muestra un candado en la pantalla de bloqueo y lo abre en blanco, junto a un check blanco, durante 0,65 segundos al desbloquear. La apertura comienza al detectar el cambio del estado de sesión, sin reconstruir la ventana ni esperar al aviso distribuido; el momento exacto depende de cuándo macOS publica ese estado. Solo se consulta mientras la pantalla bloqueada está encendida. La ventana de bloqueo es independiente, no recibe clics ni teclas y solo contiene el indicador. Usa funciones privadas de SkyLight cargadas opcionalmente (referencia: [SkyLightWindow](https://github.com/Lakr233/SkyLightWindow)); si no están disponibles, conserva la animación al regresar al escritorio. Requiere que Buddy esté abierto en la sesión: no se muestra durante el arranque de FileVault ni antes del primer inicio de sesión.

Para previsualizar en Debug, iniciar con `BUDDY_DEBUG_NOTCH=lock` o `BUDDY_DEBUG_NOTCH=unlock`. Estas opciones simulan el indicador sin bloquear el Mac. La validación visual final se hace bloqueando con Control–Command–Q y desbloqueando normalmente.
