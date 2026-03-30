use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ComicbookInfo {
    pub id_comicbook_info: i64,
    pub titulo: String,
    pub id_volume: Option<i64>,
    pub numero: Option<String>,
    pub resumen: Option<String>,
    pub calificacion: Option<f64>,
    pub id_comicvine: Option<i64>,
    pub url_api_detalle: Option<String>,
    pub fue_actualizado_api: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewComicbookInfo {
    pub titulo: String,
    pub id_volume: Option<i64>,
    pub numero: Option<String>,
    pub resumen: Option<String>,
    pub calificacion: Option<f64>,
    pub id_comicvine: Option<i64>,
    pub url_api_detalle: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComicbookInfoView {
    pub info: ComicbookInfo,
    pub physical_count: i64,
    pub ruta_cover: Option<String>,
    pub id_comicbook: Option<i64>,
    pub url_original: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ComicbookInfoCover {
    pub id: i64,
    pub id_comicbook_info: i64,
    pub url_original: String,
    pub ruta_local: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewComicbookInfoCover {
    pub id_comicbook_info: i64,
    pub url_original: String,
    pub ruta_local: Option<String>,
}
