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
    pub carpeta_catalogo_normalizado: Option<String>,
    pub intervalo_api: f64,
    pub api_url: Option<String>,
    pub clip_al_arranque: bool,
    /// Calcula los embeddings CLIP al terminar de bajar o actualizar un volumen.
    pub clip_al_descargar: bool,
    pub reader_filter: i64,
    pub catalog_filter: String,
    pub volume_update_filter: String,
    pub volume_sort_order: String,
    /// Ejecutable del lector elegido por el usuario; `None` usa la búsqueda automática.
    pub reader_path: Option<String>,
    /// Ver [`ReaderFit`].
    pub reader_fit: String,
    /// Ver [`ReaderDirection`].
    pub reader_direction: String,
    /// Ver [`ReaderGuided`].
    pub reader_guided: String,
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
            carpeta_catalogo_normalizado: None,
            intervalo_api: 0.5,
            api_url: Some("https://comicvine.gamespot.com/api/".to_string()),
            clip_al_arranque: true,
            clip_al_descargar: true,
            reader_filter: 3,
            catalog_filter: "all".to_string(),
            volume_update_filter: "all".to_string(),
            volume_sort_order: "nombre_asc".to_string(),
            reader_path: None,
            reader_fit: ReaderFit::default().as_str().to_string(),
            reader_direction: ReaderDirection::default().as_str().to_string(),
            reader_guided: ReaderGuided::default().as_str().to_string(),
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

/// Declara una opción del lector guardada como texto en `setups`.
/// El mismo texto se pasa tal cual a `cosmic-reader` por línea de comandos.
macro_rules! reader_option {
    ($(#[$meta:meta])* $name:ident { $($(#[$vmeta:meta])* $variant:ident => $value:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
        pub enum $name {
            $($(#[$vmeta])* $variant),+
        }

        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $value),+
                }
            }

            /// Valores desconocidos vuelven al predeterminado.
            pub fn from_db(value: &str) -> Self {
                match value {
                    $($value => Self::$variant,)+
                    _ => Self::default(),
                }
            }
        }
    };
}

reader_option!(
    /// Cómo se ajusta la página a la ventana del lector.
    ReaderFit {
        #[default]
        Page => "page",
        Height => "height",
        Width => "width",
        Original => "original",
    }
);

reader_option!(
    /// Sentido de lectura: `Rtl` invierte las flechas (manga).
    ReaderDirection {
        #[default]
        Ltr => "ltr",
        Rtl => "rtl",
    }
);

reader_option!(
    /// Qué hacer cuando el cómic tiene viñetas para lectura guiada.
    ReaderGuided {
        #[default]
        Ask => "ask",
        Always => "always",
        Never => "never",
    }
);
