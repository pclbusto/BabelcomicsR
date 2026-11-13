use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const CONFIG_FILE: &str = "cli_config.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReplConfig {
    /// Columnas seleccionadas para cada entidad: key = "comics", value = ["id", "path", "titulo", ...]
    pub columns: HashMap<String, Vec<String>>,
}

impl ReplConfig {
    pub fn load() -> Self {
        let path = config_path();
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Ok(cfg) = serde_json::from_str(&data) {
                return cfg;
            }
        }
        Self::default()
    }

    pub fn save(&self) -> Result<()> {
        let path = config_path();
        let data = serde_json::to_string_pretty(self)?;
        std::fs::write(path, data)?;
        Ok(())
    }

    pub fn get_columns(&self, entity: &str) -> Vec<String> {
        self.columns
            .get(entity)
            .cloned()
            .unwrap_or_else(|| Self::default_columns(entity))
    }

    pub fn set_columns(&mut self, entity: &str, cols: Vec<String>) {
        self.columns.insert(entity.to_string(), cols);
    }

    pub fn default_columns(entity: &str) -> Vec<String> {
        match entity {
            "comics" => vec![
                "id_comicbook".to_string(),
                "path".to_string(),
                "titulo".to_string(),
                "numero".to_string(),
                "nombre_volume".to_string(),
                "nombre_publisher".to_string(),
                "calificacion".to_string(),
                "en_papelera".to_string(),
            ],
            "comicbook_info" => vec![
                "id_comicbook_info".to_string(),
                "titulo".to_string(),
                "numero".to_string(),
                "calificacion".to_string(),
                "id_comicvine".to_string(),
                "fue_actualizado_api".to_string(),
            ],
            "comicbook_detail" => vec![
                "id_detail".to_string(),
                "comicbook_id".to_string(),
                "indice_pagina".to_string(),
                "orden_pagina".to_string(),
                "tipo_pagina".to_string(),
                "nombre_pagina".to_string(),
            ],
            "volumes" => vec![
                "id_volume".to_string(),
                "nombre".to_string(),
                "anio_inicio".to_string(),
                "cantidad_numeros".to_string(),
                "nombre_publisher".to_string(),
            ],
            "publishers" => vec![
                "id_publisher".to_string(),
                "nombre".to_string(),
                "descripcion".to_string(),
                "id_comicvine".to_string(),
            ],
            _ => vec!["*".to_string()],
        }
    }
}

fn config_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let dir = std::path::PathBuf::from(format!("{}/.local/share/babelcomics", home));
    let _ = std::fs::create_dir_all(&dir);
    dir.join(CONFIG_FILE)
}

/// Genera un hyperlink OSC 8 para la terminal
/// En terminales compatibles (GNOME Terminal, iTerm2, Kitty, etc.),
/// el texto será clickeable y abrirá el file manager en el directorio.
pub fn osc8_link(text: &str, file_path: &str) -> String {
    // Construir URL file://
    let url = format!("file://{}", file_path);
    // OSC 8: \e]8;;URL\e\\TEXT\e]8;;\e\\
    format!("\x1b]8;;{}\x1b\\{}\x1b]8;;\x1b\\", url, text)
}

/// Lista todas las entidades disponibles
pub fn all_entities() -> Vec<(&'static str, &'static str)> {
    vec![
        ("comics", "Archivos de comics fisicos"),
        ("comicbook_info", "Metadatos ComicVine de comics"),
        ("comicbook_detail", "Paginas internas de comics"),
        ("volumes", "Series / Volúmenes"),
        ("publishers", "Editoriales"),
    ]
}

/// Lista todas las columnas disponibles por entidad
pub fn available_columns(entity: &str) -> Vec<(&'static str, &'static str)> {
    match entity {
        "comics" => vec![
            ("id_comicbook", "ID del comic"),
            ("path", "Ruta del archivo"),
            ("en_papelera", "Esta en papelera (bool)"),
            ("calidad", "Calidad del scan"),
            ("procesado", "Fue procesado (bool)"),
            ("error_ultimo_escaneo", "Error del ultimo escaneo"),
            ("titulo", "Titulo (de ComicVine)"),
            ("numero", "Numero de issue"),
            ("calificacion", "Calificacion (0-10)"),
            ("nombre_volume", "Nombre del volumen"),
            ("nombre_publisher", "Nombre de la editorial"),
        ],
        "comicbook_info" => vec![
            ("id_comicbook_info", "ID del metadato"),
            ("titulo", "Titulo"),
            ("id_volume", "ID del volumen"),
            ("numero", "Numero de issue"),
            ("resumen", "Resumen"),
            ("calificacion", "Calificacion"),
            ("id_comicvine", "ID ComicVine"),
            ("url_api_detalle", "URL API detalle"),
            ("fue_actualizado_api", "Fue actualizado desde API"),
        ],
        "comicbook_detail" => vec![
            ("id_detail", "ID del detalle"),
            ("comicbook_id", "ID del comic"),
            ("indice_pagina", "Indice de pagina"),
            ("orden_pagina", "Orden de pagina"),
            ("tipo_pagina", "Tipo de pagina (0-5)"),
            ("nombre_pagina", "Nombre de pagina"),
        ],
        "volumes" => vec![
            ("id_volume", "ID del volumen"),
            ("nombre", "Nombre"),
            ("deck", "Resumen breve"),
            ("anio_inicio", "Anio de inicio"),
            ("cantidad_numeros", "Cantidad de numeros"),
            ("nombre_publisher", "Editorial"),
        ],
        "publishers" => vec![
            ("id_publisher", "ID de la editorial"),
            ("nombre", "Nombre"),
            ("descripcion", "Descripcion"),
            ("id_comicvine", "ID de ComicVine"),
        ],
        _ => vec![],
    }
}
