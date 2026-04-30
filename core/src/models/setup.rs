use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Setup {
    pub setupkey: String,
    pub api_key_encrypted: Option<String>,
    pub modo_oscuro: bool,
    pub thumbnail_size: i64,
    pub items_por_pagina: i64,
    pub num_workers: i64,
    pub idioma: Option<String>,
    pub carpeta_thumbnails: Option<String>,
    pub intervalo_api: f64,
    pub api_url: Option<String>,
    pub clip_al_arranque: bool,
    pub reader_filter: i64,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            setupkey: "default".to_string(),
            api_key_encrypted: None,
            modo_oscuro: false,
            thumbnail_size: 200,
            items_por_pagina: 50,
            num_workers: 4,
            idioma: None,
            carpeta_thumbnails: None,
            intervalo_api: 0.5,
            api_url: Some("https://comicvine.gamespot.com/api/".to_string()),
            clip_al_arranque: true,
            reader_filter: 3,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct SetupDirectorio {
    pub id: i64,
    pub path: String,
    pub setup_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewSetupDirectorio {
    pub path: String,
    pub setup_key: String,
}
