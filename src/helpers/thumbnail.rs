use std::path::Path;

use anyhow::{Context, Result};
use image::{imageops::FilterType, ImageFormat};

/// Los tres tamaños de card soportados por el sistema.
/// Cada variante define las dimensiones exactas de la card en la UI
/// y el tamaño del thumbnail que se genera en disco.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardSize {
    Small,
    Medium,
    Large,
}

impl CardSize {
    /// Dimensiones (ancho, alto) en píxeles — iguales para card y thumbnail.
    pub fn dims(self) -> (u32, u32) {
        match self {
            CardSize::Small  => (160, 220),
            CardSize::Medium => (240, 320),
            CardSize::Large  => (320, 430),
        }
    }
}

impl Default for CardSize {
    fn default() -> Self {
        CardSize::Medium
    }
}

impl CardSize {
    /// Convierte el valor guardado en BD a CardSize.
    /// Valores legacy (ej: 200 píxeles) caen en Medium.
    pub fn from_db(val: i64) -> Self {
        match val {
            0 => CardSize::Small,
            2 => CardSize::Large,
            _ => CardSize::Medium,
        }
    }

    pub fn to_db(self) -> i64 {
        match self {
            CardSize::Small  => 0,
            CardSize::Medium => 1,
            CardSize::Large  => 2,
        }
    }

    /// Índice en el ComboRow de preferencias (Small=0, Medium=1, Large=2).
    pub fn combo_index(self) -> u32 {
        self.to_db() as u32
    }

    pub fn from_combo_index(idx: u32) -> Self {
        match idx {
            0 => CardSize::Small,
            2 => CardSize::Large,
            _ => CardSize::Medium,
        }
    }

    /// Subdirectorio usado para aislar thumbnails por tamaño.
    pub fn dir_name(self) -> &'static str {
        match self {
            CardSize::Small  => "small",
            CardSize::Medium => "medium",
            CardSize::Large  => "large",
        }
    }
}

/// Genera los tres tamaños de thumbnail (Small, Medium, Large) a partir de los bytes originales.
/// Esto es MUCHO más eficiente que extraer el archivo tres veces.
pub fn generate_all_thumbnails(image_bytes: &[u8], id_comicbook: i64) -> Result<()> {
    let img = image::load_from_memory(image_bytes)
        .context("No se pudo decodificar la imagen original")?;

    for size in [CardSize::Small, CardSize::Medium, CardSize::Large] {
        let (_, h) = size.dims();
        // Forzamos que la altura sea exacta (h) permitiendo que el ancho crezca libremente.
        // Esto evita que las portadas apaisadas (landscape) queden más bajas que las verticales.
        let thumbnail = img.resize(u32::MAX, h, FilterType::Lanczos3);
        
        let output_path = crate::helpers::paths::comic_thumbnail_path(id_comicbook, size);
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        thumbnail
            .save_with_format(&output_path, ImageFormat::Jpeg)
            .with_context(|| format!("No se pudo guardar thumbnail {} en: {}", size.dir_name(), output_path.display()))?;
    }

    Ok(())
}

/// Genera un thumbnail a partir de bytes de imagen y lo guarda en `output_path`.
/// La imagen se escala para tener exactamente la altura de `size`, manteniendo
/// el aspecto original. Siempre se guarda como JPEG.
pub fn generate_thumbnail(image_bytes: &[u8], output_path: &Path, size: CardSize) -> Result<()> {
    let img = image::load_from_memory(image_bytes)
        .context("No se pudo decodificar la imagen")?;

    let (_, h) = size.dims();
    let thumbnail = img.resize(u32::MAX, h, FilterType::Lanczos3);

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    thumbnail
        .save_with_format(output_path, ImageFormat::Jpeg)
        .with_context(|| format!("No se pudo guardar thumbnail en: {}", output_path.display()))?;

    Ok(())
}

/// Comprueba si ya existe el thumbnail para un comic.
pub fn thumbnail_exists(path: &Path) -> bool {
    path.exists()
}
