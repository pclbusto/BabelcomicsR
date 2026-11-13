use std::path::Path;
use walkdir::WalkDir;

/// Extensiones de comic soportadas
pub const COMIC_EXTENSIONS: &[&str] = &["cbz", "cbr", "cb7", "zip", "rar", "7z", "pdf"];

/// Escanea un directorio recursivamente y devuelve todos los archivos de comic encontrados.
pub fn scan_directory(dir: &str) -> Vec<String> {
    let extensions = COMIC_EXTENSIONS
        .iter()
        .map(|extension| (*extension).to_string())
        .collect::<Vec<_>>();
    scan_directory_with_extensions(dir, &extensions)
}

/// Escanea un directorio usando únicamente las extensiones habilitadas.
pub fn scan_directory_with_extensions(dir: &str, extensions: &[String]) -> Vec<String> {
    WalkDir::new(dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| has_enabled_extension(e.path(), extensions))
        .map(|e| e.path().to_string_lossy().to_string())
        .collect()
}

/// Escanea múltiples directorios en paralelo y devuelve todos los archivos encontrados.
pub fn scan_directories(dirs: &[String]) -> Vec<String> {
    use rayon::prelude::*;

    dirs.par_iter()
        .flat_map(|dir| scan_directory(dir))
        .collect()
}

/// Escanea múltiples directorios respetando la selección de tipos de archivo.
pub fn scan_directories_with_extensions(dirs: &[String], extensions: &[String]) -> Vec<String> {
    use rayon::prelude::*;

    dirs.par_iter()
        .flat_map(|dir| scan_directory_with_extensions(dir, extensions))
        .collect()
}

pub fn is_comic_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| COMIC_EXTENSIONS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

pub fn has_enabled_extension(path: &Path, extensions: &[String]) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extensions
                .iter()
                .any(|enabled| enabled.eq_ignore_ascii_case(extension))
        })
}

/// Detecta el formato del archivo por su extensión.
#[derive(Debug, Clone, PartialEq)]
pub enum ComicFormat {
    Cbz,
    Cbr,
    Cb7,
    Pdf,
    Unknown,
}

impl ComicFormat {
    pub fn from_path(path: &Path) -> Self {
        match path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .as_deref()
        {
            Some("cbz") | Some("zip") => Self::Cbz,
            Some("cbr") | Some("rar") => Self::Cbr,
            Some("cb7") | Some("7z") => Self::Cb7,
            Some("pdf") => Self::Pdf,
            _ => Self::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabled_extensions_are_case_insensitive() {
        let enabled = vec!["cbz".to_string(), "cbr".to_string()];

        assert!(has_enabled_extension(Path::new("issue.CBZ"), &enabled));
        assert!(has_enabled_extension(Path::new("issue.cbr"), &enabled));
        assert!(!has_enabled_extension(Path::new("issue.pdf"), &enabled));
        assert!(!has_enabled_extension(Path::new("issue.zip"), &enabled));
    }

    #[test]
    fn no_extensions_means_no_files() {
        assert!(!has_enabled_extension(Path::new("issue.cbz"), &[]));
    }
}
