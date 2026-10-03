//! Which files the server may read: the documents folder (`--out`), the skills folder and every `--read <dir>`
//! Buddy passed (the attachments folder and the folders the user authorized). Nothing else.
//!
//! A path is allowed only when its canonical form (every link resolved, every `..` gone) lies inside the canonical
//! form of one of those folders, so neither `../` nor a symbolic link can lead out of them.

use std::path::{Path, PathBuf};

use super::OfficeError;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Access {
    /// Where a relative path is taken from: the documents folder.
    pub base: PathBuf,
    /// The folders that may be read, as given (canonicalized on every check: they may appear later).
    pub roots: Vec<PathBuf>,
}

impl Access {
    pub fn new(base: &Path, roots: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut all = vec![base.to_path_buf()];
        for root in roots {
            if !all.contains(&root) {
                all.push(root);
            }
        }
        Access { base: base.to_path_buf(), roots: all }
    }

    /// The folders as the agent should see them.
    pub fn describe(&self) -> String {
        self.roots.iter().map(|r| r.display().to_string()).collect::<Vec<_>>().join(", ")
    }

    /// The canonical path of the regular file `raw` names, if it lies inside one of the folders.
    pub fn resolve(&self, raw: &str) -> Result<PathBuf, OfficeError> {
        let raw = raw.trim();
        let raw = raw.strip_prefix("file://").unwrap_or(raw);
        if raw.is_empty() {
            return Err(OfficeError::new("Falta la ruta del archivo (path)."));
        }
        let path = match raw.strip_prefix("~/") {
            Some(rest) => match std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
                Some(home) => PathBuf::from(home).join(rest),
                None => PathBuf::from(raw),
            },
            None => PathBuf::from(raw),
        };
        let path = if path.is_absolute() { path } else { self.base.join(path) };
        let canonical = path.canonicalize().map_err(|_| OfficeError(format!("No encuentro el archivo {raw}.")))?;
        let inside = self.roots.iter().filter_map(|r| r.canonicalize().ok()).any(|root| canonical.starts_with(&root));
        if !inside {
            return Err(OfficeError(format!(
                "No tengo permiso para leer {raw}. Solo puedo leer archivos dentro de: {}.",
                self.describe()
            )));
        }
        if !canonical.is_file() {
            return Err(OfficeError(format!("{raw} no es un archivo.")));
        }
        Ok(canonical)
    }

    /// The bytes of the allowed file `raw`, refusing anything over `limit` bytes.
    pub fn read(&self, raw: &str, limit: u64) -> Result<(PathBuf, Vec<u8>), OfficeError> {
        let path = self.resolve(raw)?;
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(u64::MAX);
        if size > limit {
            return Err(OfficeError(format!("{raw} es demasiado grande (máximo {} MB).", limit / (1024 * 1024))));
        }
        let data = std::fs::read(&path).map_err(|_| OfficeError(format!("No pude leer {raw}.")))?;
        Ok((path, data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::office::test_support::TempDir;

    #[test]
    fn only_files_inside_the_folders_are_allowed() {
        let tmp = TempDir::new("access");
        let out = tmp.0.join("documentos");
        let shared = tmp.0.join("adjuntos");
        let secret = tmp.0.join("privado");
        for dir in [&out, &shared, &secret] {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(out.join("informe.txt"), "a").unwrap();
        std::fs::write(shared.join("nota.txt"), "b").unwrap();
        std::fs::write(secret.join("clave.txt"), "c").unwrap();
        let access = Access::new(&out, [shared.clone()]);

        assert_eq!(access.resolve("informe.txt").unwrap(), out.join("informe.txt").canonicalize().unwrap(), "relative: the documents folder");
        assert!(access.resolve(&shared.join("nota.txt").to_string_lossy()).is_ok());
        assert!(access.resolve(&secret.join("clave.txt").to_string_lossy()).is_err());
        assert!(access.resolve("../privado/clave.txt").is_err(), "no way up");
        assert!(access.resolve(&shared.join("../privado/clave.txt").to_string_lossy()).is_err());
        assert!(access.resolve("").is_err());
        assert!(access.resolve("no-existe.txt").is_err());
        assert!(access.resolve(&shared.to_string_lossy()).is_err(), "a folder is not a file");
        let (_, data) = access.read("informe.txt", 10).unwrap();
        assert_eq!(data, b"a");
        assert!(access.read("informe.txt", 0).is_err(), "over the limit");
    }

    #[cfg(unix)]
    #[test]
    fn a_link_out_of_the_folder_is_not_followed() {
        let tmp = TempDir::new("access-link");
        let out = tmp.0.join("documentos");
        let secret = tmp.0.join("privado");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::create_dir_all(&secret).unwrap();
        std::fs::write(secret.join("clave.txt"), "c").unwrap();
        std::os::unix::fs::symlink(secret.join("clave.txt"), out.join("enlace.txt")).unwrap();
        std::os::unix::fs::symlink(&secret, out.join("carpeta")).unwrap();
        let access = Access::new(&out, []);
        assert!(access.resolve("enlace.txt").is_err());
        assert!(access.resolve("carpeta/clave.txt").is_err());
        // A link that stays inside is fine.
        std::fs::write(out.join("real.txt"), "r").unwrap();
        std::os::unix::fs::symlink(out.join("real.txt"), out.join("dentro.txt")).unwrap();
        assert!(access.resolve("dentro.txt").is_ok());
    }
}
