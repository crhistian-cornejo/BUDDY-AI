// Handing things over to the Windows shell: URLs to the default browser, project
// folders to VS Code (or Explorer). Nothing here goes through `cmd /C`.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use windows::core::{w, PCWSTR};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// Keeps spawned helpers from flashing a console window.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn downloads_dir() -> Result<PathBuf, String> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{SHGetKnownFolderPath, FOLDERID_Downloads, KF_FLAG_DEFAULT};
    // SAFETY: a constant known-folder ID. Windows allocates the returned string; release it after copying.
    unsafe {
        let ptr = SHGetKnownFolderPath(&FOLDERID_Downloads, KF_FLAG_DEFAULT, None)
            .map_err(|_| "No se pudo localizar la carpeta Descargas.")?;
        let text = ptr.to_string();
        CoTaskMemFree(Some(ptr.0.cast()));
        let path = PathBuf::from(text.map_err(|_| "La ruta de Descargas no se pudo leer.")?);
        if !path.is_absolute() { return Err("La ruta de Descargas no es absoluta.".into()); }
        std::fs::create_dir_all(&path).map_err(|_| "No se pudo preparar la carpeta Descargas.")?;
        Ok(path)
    }
}

/// An http(s) URL of sane size with no spaces or control characters. The URL
/// comes from the webview (and, further back, from API responses).
fn is_safe_web_url(url: &str) -> bool {
    (url.starts_with("https://") || url.starts_with("http://"))
        && url.len() <= 2048
        && !url.chars().any(|c| c.is_control() || c.is_whitespace())
}

/// Opens an http(s) URL in the default browser. Anything else is ignored.
pub fn open_url(url: &str) {
    if !is_safe_web_url(url) {
        return;
    }
    let wide: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: the string is NUL-terminated and outlives the call; every other
    // argument is null or a constant.
    unsafe {
        ShellExecuteW(None, w!("open"), PCWSTR(wide.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

/// Only an absolute path to an existing folder is handed to another program. The
/// value comes from the webview (originally from a hook payload), and something
/// like `--install-extension x` must never reach `code` as a flag.
fn is_plain_folder(p: &str) -> bool {
    let path = Path::new(p);
    path.is_absolute() && path.is_dir()
}

/// "Open terminal" opens the working folder in VS Code when `code` is on PATH,
/// and falls back to Explorer otherwise. Returns true when VS Code was launched.
pub fn open_in_vscode(path: Option<&str>) -> bool {
    // No `cmd /C` anywhere near this. The path is a project folder chosen by
    // whoever is using Claude Code, and cmd would happily read `&`, `^` and `%`
    // in a folder name as syntax. Finding the launcher ourselves and handing the
    // path over as a separate argument keeps it a path.
    let path = path.filter(|p| !p.is_empty() && is_plain_folder(p));
    if let Some(code) = find_on_path("code") {
        let mut cmd = Command::new(code);
        if let Some(p) = path {
            cmd.arg(p);
        }
        if cmd.creation_flags(CREATE_NO_WINDOW).spawn().is_ok() {
            return true;
        }
    }
    if let Some(p) = path {
        let _ = Command::new("explorer").arg(p).spawn();
    }
    false
}

/// Our own `where`: walks %PATH% against %PATHEXT%, no shell involved.
/// Rust quotes arguments correctly for `.cmd`/`.bat` targets since 1.77, so
/// spawning `code.cmd` directly is safe.
pub fn find_on_path(stem: &str) -> Option<PathBuf> {
    let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    let dirs = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&dirs) {
        for ext in exts.split(';').filter(|e| !e.is_empty()) {
            let candidate = dir.join(format!("{stem}{}", ext.to_lowercase()));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Only called with a path resolved from the index or a validated session folder.
pub fn open_local(path: &Path, reveal: bool) -> Result<(), String> {
    if !path.is_absolute() || !path.exists() { return Err("La ruta no existe.".into()); }
    if reveal {
        Command::new("explorer.exe").arg(format!("/select,{}", path.display())).creation_flags(CREATE_NO_WINDOW).spawn().map_err(|_| "No se pudo abrir el Explorador.")?;
        return Ok(());
    }
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
    if ["rs", "ts", "tsx", "js", "jsx", "py", "ps1", "cmd", "bat", "sh", "cs", "cpp", "c", "swift", "json", "toml", "yml", "yaml", "html", "css", "sql"].contains(&ext.as_str()) {
        if let Some(code) = find_on_path("code") {
            Command::new(code).arg("--").arg(path).creation_flags(CREATE_NO_WINDOW).spawn().map_err(|_| "No se pudo abrir el archivo en VS Code.")?;
            return Ok(());
        }
        Command::new("notepad.exe").arg(path).spawn().map_err(|_| "No se pudo abrir el archivo.")?;
        return Ok(());
    }
    let wide: Vec<u16> = path.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();
    // SAFETY: a validated indexed path, passed as the filename, not as shell syntax.
    let result = unsafe { ShellExecuteW(None, w!("open"), PCWSTR(wide.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
    if result.0 as usize <= 32 { return Err("Windows no tiene una aplicación asociada a este archivo.".into()); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_web_urls_are_opened() {
        assert!(is_safe_web_url("https://dashboard.stripe.com/payments"));
        assert!(is_safe_web_url("http://localhost:3000/a?b=c#d"));
        for bad in [
            "",
            "file:///C:/Windows/System32/calc.exe",
            "javascript:alert(1)",
            "ms-msdt:/id PCWDiagnostic",
            "https://example.com/a b",
            "https://example.com/\r\nX: y",
            "C:\\Windows\\notepad.exe",
        ] {
            assert!(!is_safe_web_url(bad), "{bad:?} must be refused");
        }
        assert!(!is_safe_web_url(&format!("https://e.com/{}", "a".repeat(2100))));
    }

    #[test]
    fn a_flag_is_never_passed_off_as_a_folder() {
        assert!(!is_plain_folder("--install-extension"));
        assert!(!is_plain_folder("-r"));
        assert!(!is_plain_folder("relative\\dir"));
        assert!(!is_plain_folder(r"C:\definitely\not\here"));
        assert!(is_plain_folder(std::env::temp_dir().to_str().unwrap()));
    }
}
