use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Volume {
    pub id_volume: i64,
    pub nombre: String,
    pub deck: String,
    pub descripcion: String,
    pub url: String,
    pub image_url: String,
    pub id_publisher: i64,
    pub anio_inicio: i64,
    pub cantidad_numeros: i64,
    pub id_comicvine: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewVolume {
    pub nombre: String,
    pub deck: String,
    pub descripcion: String,
    pub url: String,
    pub image_url: String,
    pub id_publisher: i64,
    pub anio_inicio: i64,
    pub cantidad_numeros: i64,
    pub id_comicvine: Option<i64>,
}

/// Vista para la rejilla de volúmenes con datos agregados
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeView {
    pub id_volume: i64,
    pub nombre: String,
    pub anio_inicio: i64,
    pub cantidad_numeros: i64,             // Total en ComicVine
    pub cantidad_poseida: i64,             // Cuántos archivos tenemos
    pub id_comicbook_portada: Option<i64>, // ID del primer comic para el thumbnail
    pub path_portada: Option<String>,      // Path físico del comic de portada
    pub image_url: String,                 // URL de la imagen en ComicVine
}
