use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Publisher {
    pub id_publisher: i64,
    pub nombre: String,
    pub descripcion: Option<String>,
    pub id_comicvine: Option<i64>,
    pub image_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewPublisher {
    pub nombre: String,
    pub descripcion: Option<String>,
    pub id_comicvine: Option<i64>,
    pub image_url: Option<String>,
}
