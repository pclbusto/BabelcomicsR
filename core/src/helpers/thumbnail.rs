use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use image::{ImageFormat, imageops::FilterType};

/// Algoritmo de escalado usado para los thumbnails del lector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReaderFilter {
    Nearest,
    Triangle,
    CatmullRom,
    #[default]
    Lanczos3,
}

impl ReaderFilter {
    pub fn from_db(val: i64) -> Self {
        match val {
            0 => Self::Nearest,
            1 => Self::Triangle,
            2 => Self::CatmullRom,
            _ => Self::Lanczos3,
        }
    }

    pub fn to_db(self) -> i64 {
        match self {
            Self::Nearest    => 0,
            Self::Triangle   => 1,
            Self::CatmullRom => 2,
            Self::Lanczos3   => 3,
        }
    }

    pub fn combo_index(self) -> u32 { self.to_db() as u32 }
    pub fn from_combo_index(idx: u32) -> Self { Self::from_db(idx as i64) }

    fn to_filter_type(self) -> FilterType {
        match self {
            Self::Nearest    => FilterType::Nearest,
            Self::Triangle   => FilterType::Triangle,
            Self::CatmullRom => FilterType::CatmullRom,
            Self::Lanczos3   => FilterType::Lanczos3,
        }
    }
}

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
            CardSize::Small => (160, 220),
            CardSize::Medium => (240, 320),
            CardSize::Large => (320, 430),
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
            CardSize::Small => 0,
            CardSize::Medium => 1,
            CardSize::Large => 2,
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
            CardSize::Small => "small",
            CardSize::Medium => "medium",
            CardSize::Large => "large",
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
            .with_context(|| {
                format!(
                    "No se pudo guardar thumbnail {} en: {}",
                    size.dir_name(),
                    output_path.display()
                )
            })?;
    }

    Ok(())
}

/// Genera un thumbnail a partir de bytes de imagen y lo guarda en `output_path`.
/// La imagen se escala para tener exactamente la altura de `size`, manteniendo
/// el aspecto original. Siempre se guarda como JPEG.
pub fn generate_thumbnail(image_bytes: &[u8], output_path: &Path, size: CardSize) -> Result<()> {
    let img = image::load_from_memory(image_bytes).context("No se pudo decodificar la imagen")?;

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

// ── Utilidades para la UI ─────────────────────────────────────────────────────

/// Escala una imagen al `height` indicado (píxeles) manteniendo la relación
/// de aspecto y devuelve los píxeles RGB crudos listos para un GdkPixbuf.
/// Retorna `(data, width, height, rowstride)`.
pub fn resize_to_height_rgb(bytes: &[u8], height: u32) -> Option<(Vec<u8>, i32, i32, i32)> {
    let img = image::load_from_memory(bytes).ok()?;
    let scaled = img.resize(u32::MAX, height, FilterType::Lanczos3);
    drop(img);
    let rgb = scaled.into_rgb8();
    let width = rgb.width() as i32;
    let h = rgb.height() as i32;
    let rowstride = width * 3;
    Some((rgb.into_raw(), width, h, rowstride))
}

/// Escala una imagen al `height` indicado y la devuelve codificada como JPEG.
pub fn resize_to_height_jpeg(bytes: &[u8], height: u32) -> Option<Vec<u8>> {
    let img = image::load_from_memory(bytes).ok()?;
    let scaled = img.resize(u32::MAX, height, FilterType::Lanczos3);
    drop(img);
    let mut out = Vec::new();
    scaled
        .write_to(&mut std::io::Cursor::new(&mut out), ImageFormat::Jpeg)
        .ok()?;
    Some(out)
}

/// Píxeles RGB crudos de una página escalada, listos para crear un GdkPixbuf.
pub struct PageThumb {
    pub data: Vec<u8>,
    pub width: i32,
    pub height: i32,
    pub rowstride: i32,
}

/// Ruta canónica de una página extraída dentro de `dir`.
pub fn page_path_in(page_name: &str, dir: &Path) -> PathBuf {
    let file_name = if page_name.chars().all(|c| c.is_ascii_digit()) {
        format!("page-{}.jpg", page_name)
    } else {
        std::path::Path::new(page_name)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
    };
    dir.join(file_name)
}

/// Genera o carga el thumbnail de una página de cómic y devuelve píxeles RGB
/// crudos. Devolver píxeles directamente (sin encode JPEG) evita un encode
/// costoso en el hilo bloqueante y un decode en el hilo principal de GTK.
pub async fn load_page_thumb(
    comic_path: String,
    page_name: String,
    pages_dir: PathBuf,
    thumb_path: PathBuf,
    filter: ReaderFilter,
) -> Result<PageThumb> {
    use crate::helpers::extractor::extract_page_to_memory;

    tokio::task::spawn_blocking(move || -> Result<PageThumb> {
        let cached = page_path_in(&page_name, &pages_dir);

        let rgb = if thumb_path.exists() {
            image::open(&thumb_path)
                .context("No se pudo abrir el thumbnail cacheado")?
                .into_rgb8()
        } else {
            let img = if cached.exists() {
                image::open(&cached).context("No se pudo abrir la página extraída")?
            } else {
                let bytes = extract_page_to_memory(&comic_path, &page_name)?;
                let img = image::load_from_memory(&bytes)
                    .context("No se pudo decodificar la página")?;
                drop(bytes);
                img
            };

            let thumb = img.resize(160, u32::MAX, filter.to_filter_type());
            drop(img);

            if let Some(parent) = thumb_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            thumb
                .save_with_format(&thumb_path, ImageFormat::Jpeg)
                .context("No se pudo guardar el thumbnail")?;
            thumb.into_rgb8()
        };

        let width = rgb.width() as i32;
        let height = rgb.height() as i32;
        let rowstride = width * 3;
        Ok(PageThumb {
            data: rgb.into_raw(),
            width,
            height,
            rowstride,
        })
    })
    .await?
}
