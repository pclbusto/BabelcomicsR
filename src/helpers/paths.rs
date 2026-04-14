use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};

use super::thumbnail::CardSize;

static THUMBNAILS_BASE: OnceLock<RwLock<PathBuf>> = OnceLock::new();

fn thumbnails_lock() -> &'static RwLock<PathBuf> {
    THUMBNAILS_BASE.get_or_init(|| RwLock::new(default_thumbnails_dir()))
}

fn default_thumbnails_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".local/share/babelcomics/thumbnails")
}

pub fn initialize_thumbnails_base(path: Option<String>) {
    let base = match path {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => default_thumbnails_dir(),
    };
    *thumbnails_lock().write().unwrap() = base;
}

pub fn set_thumbnails_base(path: PathBuf) {
    *thumbnails_lock().write().unwrap() = path;
}

/// Migra el contenido de la carpeta de thumbnails actual a una nueva ubicación.
/// Si la carpeta de destino ya existe y no está vacía, no hace nada para evitar sobrescribir.
pub fn migrate_thumbnails(old_path: PathBuf, new_path: PathBuf) -> std::io::Result<()> {
    if old_path == new_path {
        return Ok(());
    }

    if !old_path.exists() {
        std::fs::create_dir_all(&new_path)?;
        return Ok(());
    }

    // Intentar mover las subcarpetas principales
    let subdirs = ["comics", "volumes", "publishers", "comicbook_info", "comic_pages"];
    
    std::fs::create_dir_all(&new_path)?;

    for subdir in subdirs {
        let from = old_path.join(subdir);
        let to = new_path.join(subdir);
        
        if from.exists() {
            // Siempre usamos move_recursive: rename de directorios falla si el destino
            // ya existe o si origen y destino están en distintos filesystems (EXDEV).
            move_recursive(&from, &to)?;
        }
    }

    Ok(())
}

fn move_recursive(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    if !to.exists() {
        std::fs::create_dir_all(to)?;
    }

    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let path = entry.path();
        let dest = to.join(entry.file_name());

        if path.is_dir() {
            move_recursive(&path, &dest)?;
        } else {
            // rename falla con EXDEV si origen y destino están en distintos filesystems;
            // en ese caso copiamos y borramos.
            if let Err(e) = std::fs::rename(&path, &dest) {
                if e.kind() == std::io::ErrorKind::CrossesDevices {
                    std::fs::copy(&path, &dest)?;
                    std::fs::remove_file(&path)?;
                } else {
                    return Err(e);
                }
            }
        }
    }
    // Intentar borrar la carpeta origen si quedó vacía
    let _ = std::fs::remove_dir(from);
    Ok(())
}

pub fn thumbnails_dir() -> PathBuf {
    thumbnails_lock().read().unwrap().clone()
}

/// Tamaño del shard: cada carpeta contiene IDs en el rango [N*SHARD, (N+1)*SHARD).
/// Con 50 000 comics → ~50 carpetas de ≤ 1 000 archivos c/u.
const SHARD_SIZE: i64 = 1000;

#[inline]
fn shard_bucket(id: i64) -> i64 {
    (id / SHARD_SIZE) * SHARD_SIZE
}

/// Ruta del thumbnail de un comic para un tamaño dado.
/// Estructura: comics/{size}/{bucket}/{id}.jpg
/// donde bucket = (id / 1000) * 1000 → máximo 1 000 archivos por carpeta.
pub fn comic_thumbnail_path(id: i64, size: CardSize) -> PathBuf {
    thumbnails_dir()
        .join("comics")
        .join(size.dir_name())
        .join(shard_bucket(id).to_string())
        .join(format!("{}.jpg", id))
}


pub fn volume_thumbnail_path(id: i64) -> PathBuf {
    thumbnails_dir().join("volumes").join(format!("{}.jpg", id))
}

pub fn publisher_thumbnail_path(id: i64) -> PathBuf {
    thumbnails_dir().join("publishers").join(format!("{}.jpg", id))
}

/// Ruta del thumbnail de una página individual de un comic.
/// Estructura: thumbnails/comic_pages/{comicbook_id}/page_{indice}.jpg
pub fn comic_page_thumbnail_path(comicbook_id: i64, indice_pagina: i64) -> PathBuf {
    thumbnails_dir()
        .join("comic_pages")
        .join(comicbook_id.to_string())
        .join(format!("page_{}.jpg", indice_pagina))
}

pub fn comicbook_info_thumbnail_path(volume_name: &str, volume_id: i64, filename: &str) -> PathBuf {
    // Sanitizar el nombre del volumen eliminando caracteres problemáticos
    let sanitized_name = volume_name
        .chars()
        .filter(|&c| c != '.' && c != ':' && c != '/' && c != '\\' && c != '?' && c != '*' && c != '"' && c != '<' && c != '>' && c != '|')
        .collect::<String>();
        
    let folder_name = format!("{}_{}", sanitized_name, volume_id);
    thumbnails_dir()
        .join("comicbook_info")
        .join(folder_name)
        .join(filename)
}

/// Lee los bytes de la portada de un issue de ComicVine.
///
/// Prioridad:
/// 1. `ruta_local` si está seteada y el archivo existe.
/// 2. Ruta construida a partir de `url_original` usando [`comicbook_info_thumbnail_path`].
///
/// Devuelve `None` si ninguna fuente produce bytes válidos.
pub async fn read_comicbook_info_cover_bytes(
    ruta_local: Option<&str>,
    url_original: Option<&str>,
    volume_name: &str,
    id_volume: i64,
) -> Option<Vec<u8>> {
    // 1. Ruta local directa
    if let Some(path) = ruta_local {
        if let Ok(bytes) = tokio::fs::read(path).await {
            return Some(bytes);
        }
    }

    // 2. Adivinar por URL original (nombre exacto)
    if let Some(url) = url_original {
        let filename = url.split('/').last().unwrap_or("");
        if !filename.is_empty() {
            let thumb = comicbook_info_thumbnail_path(volume_name, id_volume, filename);
            if let Ok(bytes) = tokio::fs::read(&thumb).await {
                return Some(bytes);
            }

            // 3. Adivinar por ID (buscando archivos que empiecen por ID-)
            // Extraer el ID de ComicVine del nombre del archivo (suele ser el primer grupo de números)
            // O mejor aún, si el filename es algo como "12345-cover.jpg", el ID es 12345.
            if let Some(id_part) = filename.split('-').next() {
                if id_part.chars().all(|c| c.is_ascii_digit()) {
                    let folder = thumb.parent().unwrap();
                    if let Ok(mut entries) = tokio::fs::read_dir(folder).await {
                        let prefix = format!("{}-", id_part);
                        while let Ok(Some(entry)) = entries.next_entry().await {
                            let name = entry.file_name().to_string_lossy().to_string();
                            if name.starts_with(&prefix) {
                                if let Ok(bytes) = tokio::fs::read(entry.path()).await {
                                    return Some(bytes);
                                }
                            }
                        }
                    }
                }
            }

            // 4. Último recurso: Descargar desde la URL y guardar localmente
            if let Ok(resp) = reqwest::get(url).await {
                if let Ok(bytes) = resp.bytes().await {
                    let thumb = comicbook_info_thumbnail_path(volume_name, id_volume, filename);
                    if let Some(parent) = thumb.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = std::fs::write(&thumb, &bytes);
                    return Some(bytes.to_vec());
                }
            }
        }
    }
    None
}

/// Crea todos los directorios necesarios para thumbnails, incluyendo
/// un subdirectorio por cada tamaño de card.
pub fn ensure_thumbnail_dirs() -> std::io::Result<()> {
    for size in [CardSize::Small, CardSize::Medium, CardSize::Large] {
        std::fs::create_dir_all(thumbnails_dir().join("comics").join(size.dir_name()))?;
    }
    for subdir in &["volumes", "publishers", "comicbook_info", "comic_pages"] {
        std::fs::create_dir_all(thumbnails_dir().join(subdir))?;
    }
    Ok(())
}
