# apps/macos

App de Mac en SwiftUI/AppKit sobre `core` (UniFFI): mascota, notch, chat, voz (SpeechAnalyzer), Siri/Atajos. Proyecto con XcodeGen (`project.yml`; el `.xcodeproj` no se sube).

```bash
xcodegen
xcodebuild -project Buddy.xcodeproj -scheme Buddy -configuration Debug build
```

La fase «buddy-core (Rust + UniFFI)» del build ejecuta `scripts/build-core-macos.sh`: compila el núcleo como biblioteca estática y genera `Generated/` (enlaces Swift, ignorado en git). Necesita `cargo`.
