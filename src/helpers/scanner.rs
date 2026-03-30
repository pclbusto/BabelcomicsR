use std::path::Path;
use walkdir::WalkDir;

/// Extensiones de comic soportadas
const COMIC_EXTENSIONS: &[&str] = &["cbz", "cbr", "cb7", "zip", "rar", "7z", "pdf"];

/// Escanea un directorio recursivamente y devuelve todos los archivos de comic encontrados.
pub fn scan_directory(dir: &str) -> Vec<String> {
    WalkDir::new(dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| is_comic_file(e.path()))
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

pub fn is_comic_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| COMIC_EXTENSIONS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
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
            Some("cb7") | Some("7z")  => Self::Cb7,
            Some("pdf")               => Self::Pdf,
            _                         => Self::Unknown,
        }
    }
}
