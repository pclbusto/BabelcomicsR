/// Registro de directorios de extracción del lector.
///
/// Al abrir un cómic, el lector extrae páginas a una carpeta oculta
/// `.babelcomics/{hash}` junto al propio archivo. Al cerrar, borra esa carpeta.
///
/// Este registro persiste en disco para que, si la app se cierra de forma
/// inesperada (crash, apagón), el próximo arranque pueda limpiar los restos.
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct RegistryEntry {
    /// Ruta al archivo de cómic
    comic: String,
    /// Ruta al directorio de caché
    cache: String,
}

fn registry_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".local/share/babelcomics/extraction_registry.json")
}

fn read_registry() -> Vec<RegistryEntry> {
    let path = registry_path();
    let Ok(data) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    serde_json::from_str(&data).unwrap_or_default()
}

fn write_registry(entries: &[RegistryEntry]) {
    let path = registry_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(entries) {
        let _ = std::fs::write(&path, json);
    }
}

/// Registra un directorio de caché activo.
/// Llamar antes de empezar a extraer páginas.
pub fn register(comic_path: &str, cache_dir: &Path) {
    let mut entries = read_registry();
    // Evitar duplicados (p.ej. si se abre el mismo cómic dos veces)
    let cache_str = cache_dir.to_string_lossy().to_string();
    if !entries.iter().any(|e| e.cache == cache_str) {
        entries.push(RegistryEntry {
            comic: comic_path.to_string(),
            cache: cache_str,
        });
        write_registry(&entries);
    }
}

/// Elimina la entrada del registro y borra el directorio de caché del disco.
/// Llamar al cerrar la ventana del lector.
pub fn unregister(cache_dir: &Path) {
    let cache_str = cache_dir.to_string_lossy().to_string();
    let mut entries = read_registry();
    entries.retain(|e| e.cache != cache_str);
    write_registry(&entries);
    let _ = std::fs::remove_dir_all(cache_dir);
}

/// Limpia directorios huérfanos que quedaron de sesiones anteriores (crashes).
/// Llamar al arrancar la app, antes de mostrar la UI.
pub fn cleanup_stale() {
    let entries = read_registry();
    if entries.is_empty() {
        return;
    }

    let mut had_stale = false;
    for entry in &entries {
        let path = Path::new(&entry.cache);
        if path.exists() {
            tracing::warn!(
                "Directorio de extracción huérfano encontrado (crash anterior): {}  comic: {}",
                entry.cache,
                entry.comic
            );
            if let Err(e) = std::fs::remove_dir_all(path) {
                tracing::error!("No se pudo limpiar {}: {}", entry.cache, e);
            } else {
                tracing::info!("Limpiado: {}", entry.cache);
                had_stale = true;
            }
        }
    }

    // Dejar el registro vacío (todas las sesiones anteriores terminaron o se limpiaron)
    write_registry(&[]);

    if had_stale {
        tracing::info!("Limpieza de arranque completada.");
    }
}
