use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Comicbook {
    pub id_comicbook: i64,
    pub path: String,
    pub id_comicbook_info: Option<i64>,
    pub calidad: Option<String>,
    pub en_papelera: bool,
    pub embedding: Option<String>, // JSON vector
    pub error_ultimo_escaneo: Option<String>,
    pub procesado: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewComicbook {
    pub path: String,
    pub id_comicbook_info: Option<i64>,
    pub calidad: Option<String>,
}

/// Vista enriquecida que combina el archivo físico con sus metadatos
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComicbookView {
    pub id_comicbook: i64,
    pub path: String,
    pub en_papelera: bool,
    pub calidad: Option<String>,
    pub error_ultimo_escaneo: Option<String>,
    pub procesado: bool,
    // De ComicbookInfo (puede ser None si no está catalogado)
    pub titulo: Option<String>,
    pub numero: Option<String>,
    pub calificacion: Option<f64>,
    // De Volume
    pub nombre_volume: Option<String>,
    // De Publisher
    pub nombre_publisher: Option<String>,
    // Ruta local de la portada
    pub ruta_cover: Option<String>,
}
/// Filtros para la búsqueda de comics
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ComicFilter {
    pub query: Option<String>,
    pub clasificado: Option<bool>,
    pub min_calidad: Option<i32>,
}
