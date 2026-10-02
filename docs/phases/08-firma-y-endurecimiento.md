# Fase 8: firma y endurecimiento

Se hace cuando haya un build para distribuir. Lo que ya se sabe:

## Mac (el dueño ya tiene Apple Developer)

1. En Xcode o en developer.apple.com, crear un certificado **Developer ID Application**.
2. Firmar con hardened runtime: `ENABLE_HARDENED_RUNTIME = YES` (MIKA ya lo tiene en `project.yml`).
3. **Notarizar:** `xcrun notarytool submit Buddy.zip --keychain-profile <perfil> --wait`, luego `xcrun stapler staple Buddy.app`.
4. Verificar: `spctl -a -vv Buddy.app` y `codesign --verify --deep --strict Buddy.app`.
5. Los ayudantes incluidos (relevo de hooks, lector de Telegram) se firman con el mismo certificado.

Referencia: `reuse/mika/` (MIKA ya tiene `scripts/release-macos.sh` con `MIKA_SIGN_IDENTITY`).

## Windows

1. Cuenta de **Azure Artifact Signing** (~$10/mes) y un perfil de certificado.
2. Configurar la firma en `tauri.conf.json` y en GitHub Actions ([guía de Tauri](https://v2.tauri.app/distribute/sign/windows/)).
3. Firmar el ejecutable, el instalador y los ayudantes.
4. **SmartScreen:** la reputación sube con semanas de instalaciones limpias; firmar siempre con el mismo perfil.

## Antivirus

- Antes de cada versión, escanear con VirusTotal y Microsoft Defender.
- Nada de: ganchos de teclado globales, inyección en procesos, descargar ejecutables, empaquetadores (UPX), scripts ocultos.
- Los modelos que se descargan (whisper, palabra de activación) se verifican con hash antes de usarlos.
