//! Motor de embeddings visuales basado en CLIP ViT-B/32.
//!
//! # Flujo
//! 1. En el primer uso, descarga los pesos desde HuggingFace Hub
//!    (~340 MB, se cachean en `~/.local/share/babelcomics/models/`).
//! 2. Pre-procesa cada imagen: resize a 224×224, normalización CLIP estándar.
//! 3. Pasa la imagen por el codificador visual de CLIP → vector de 512 f32.
//! 4. Normaliza el vector a longitud unitaria (L2).
//! 5. La similitud entre dos portadas = producto punto (coseno) entre sus vectores.
//!
//! # Uso
//! ```rust
//! let bytes = std::fs::read("portada.jpg")?;
//! let emb = clip_embedder::embed_image(&bytes)?;           // Vec<f32>, len = 512
//! let blob = clip_embedder::to_bytes(&emb);                // almacenar en SQLite BLOB
//! let emb2 = clip_embedder::from_bytes(&blob).unwrap();
//! let sim  = clip_embedder::cosine_similarity(&emb, &emb2); // ≈ 1.0 si son iguales
//! ```

use anyhow::{Context, Result};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::clip;
use lazy_static::lazy_static;
use std::sync::Mutex;

/// Dimensión del embedding visual de CLIP ViT-B/32.
pub const EMBEDDING_DIM: usize = 512;

/// Tamaño de entrada del codificador visual.
const IMAGE_SIZE: usize = 224;

/// Identificador del modelo en HuggingFace Hub.
const MODEL_ID: &str = "openai/clip-vit-base-patch32";

// Constantes de normalización estándar de CLIP
const MEAN: [f32; 3] = [0.48145466, 0.4578275, 0.40821073];
const STD: [f32; 3] = [0.26862954, 0.26130258, 0.27577711];

// ---------------------------------------------------------------------------
// Modelo singleton (carga perezosa, protegido por Mutex)
// ---------------------------------------------------------------------------

lazy_static! {
    static ref EMBEDDER: Mutex<Option<ClipEmbedder>> = Mutex::new(None);
}

/// Calcula el embedding CLIP de una imagen.
///
/// La primera llamada descarga el modelo desde HuggingFace Hub y lo carga en
/// memoria (~340 MB de descarga, ~400 MB de RAM).  Las llamadas posteriores
/// son instantáneas (el modelo queda en el singleton).
///
/// Devuelve un vector L2-normalizado de 512 componentes.
pub fn embed_image(img_bytes: &[u8]) -> Result<Vec<f32>> {
    let mut guard = EMBEDDER.lock().unwrap();
    if guard.is_none() {
        *guard = Some(ClipEmbedder::load()?);
    }
    guard.as_ref().unwrap().embed(img_bytes)
}

/// Verifica si el modelo CLIP ya está cargado en memoria (sin descargarlo).
pub fn is_model_loaded() -> bool {
    EMBEDDER.lock().unwrap().is_some()
}

// ---------------------------------------------------------------------------
// Aritmética de vectores
// ---------------------------------------------------------------------------

/// Similitud coseno entre dos vectores L2-normalizados.
/// Rango: [-1.0, 1.0].  Valores cercanos a 1.0 = imágenes muy similares.
#[inline]
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(
        a.len(),
        b.len(),
        "Los embeddings deben tener la misma dimensión"
    );
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

// ---------------------------------------------------------------------------
// Serialización ↔ SQLite BLOB
// ---------------------------------------------------------------------------

/// Serializa un embedding a bytes (512 × f32 little-endian = 2048 bytes).
pub fn to_bytes(emb: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(emb.len() * 4);
    for &f in emb {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

/// Deserializa bytes a un embedding.  Devuelve `None` si la longitud no es múltiplo de 4.
pub fn from_bytes(bytes: &[u8]) -> Option<Vec<f32>> {
    if bytes.len() % 4 != 0 {
        return None;
    }
    Some(
        bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
    )
}

// ---------------------------------------------------------------------------
// Implementación interna
// ---------------------------------------------------------------------------

struct ClipEmbedder {
    model: clip::ClipModel,
    device: Device,
}

impl ClipEmbedder {
    fn load() -> Result<Self> {
        let device = Device::Cpu;

        // Directorio de caché: ~/.local/share/babelcomics/models/
        let models_dir = {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            std::path::PathBuf::from(home).join(".local/share/babelcomics/models")
        };
        std::fs::create_dir_all(&models_dir)
            .context("No se pudo crear el directorio de modelos")?;

        tracing::info!("Cargando modelo CLIP (caché: {})…", models_dir.display());

        // Construir la API de HuggingFace Hub con caché local
        let api = hf_hub::api::sync::ApiBuilder::new()
            .with_cache_dir(models_dir)
            .build()
            .context("Error inicializando HuggingFace Hub API")?;

        let repo = api.model(MODEL_ID.to_string());

        // Intentar safetensors primero (más rápido de mapear);
        // si no existe en el repo, bajar el pytorch_model.bin original (605 MB).
        let model_path = repo
            .get("model.safetensors")
            .or_else(|_| repo.get("pytorch_model.bin"))
            .context(
                "Error descargando pesos del modelo CLIP (model.safetensors o pytorch_model.bin)",
            )?;

        tracing::info!("Pesos descargados, cargando modelo CLIP en memoria…");

        // Configuración hardcodeada para ViT-B/32 (sin necesidad de config.json)
        let config = clip::ClipConfig::vit_base_patch32();

        // Elegir el loader según la extensión del archivo descargado
        let is_safetensors = model_path
            .extension()
            .and_then(|e| e.to_str())
            .map_or(false, |e| e == "safetensors");

        let vb = if is_safetensors {
            unsafe {
                VarBuilder::from_mmaped_safetensors(&[model_path], DType::F32, &device)
                    .context("Error cargando safetensors del modelo CLIP")?
            }
        } else {
            VarBuilder::from_pth(model_path, DType::F32, &device)
                .context("Error cargando pytorch_model.bin del modelo CLIP")?
        };

        let model = clip::ClipModel::new(vb, &config).context("Error inicializando modelo CLIP")?;

        tracing::info!("Modelo CLIP ViT-B/32 listo");

        Ok(Self { model, device })
    }

    fn embed(&self, img_bytes: &[u8]) -> Result<Vec<f32>> {
        let tensor = self
            .preprocess(img_bytes)
            .context("Error preprocesando imagen para CLIP")?;

        // Extrae features visuales → [1, projection_dim=512]
        let features = self
            .model
            .get_image_features(&tensor)
            .context("Error en inferencia CLIP")?;

        // Normalización L2 (candle la implementa directamente)
        let normalized =
            clip::div_l2_norm(&features).context("Error normalizando embedding CLIP")?;

        // Extraer como Vec<f32>: flatten [1, 512] → [512]
        let embedding: Vec<f32> = normalized
            .flatten_all()
            .context("flatten")?
            .to_vec1()
            .context("Error convirtiendo tensor CLIP a Vec<f32>")?;

        Ok(embedding)
    }

    /// Pre-procesa la imagen para el codificador visual de CLIP.
    ///
    /// Pipeline:
    /// 1. Decodificar cualquier formato soportado por `image`.
    /// 2. Redimensionar rellenando hasta exactamente 224×224 (resize_to_fill).
    /// 3. Convertir a RGB u8 plano.
    /// 4. Normalizar con media/std estándar de CLIP.
    /// 5. Construir tensor [1, 3, 224, 224] (batch, canal, alto, ancho).
    fn preprocess(&self, img_bytes: &[u8]) -> Result<Tensor> {
        let img = image::load_from_memory(img_bytes).context("No se pudo decodificar la imagen")?;

        // Resize a 224×224 con relleno central (preserva aspecto mejor que stretch)
        let img = img.resize_to_fill(
            IMAGE_SIZE as u32,
            IMAGE_SIZE as u32,
            image::imageops::FilterType::Triangle,
        );

        let img = img.into_rgb8();
        let raw = img.into_raw(); // layout: [H, W, C]

        let pixels = IMAGE_SIZE * IMAGE_SIZE; // 50 176
        let mut data = vec![0f32; 3 * pixels]; // layout planar: [C, H*W]

        for i in 0..pixels {
            let r = raw[3 * i] as f32 / 255.0;
            let g = raw[3 * i + 1] as f32 / 255.0;
            let b = raw[3 * i + 2] as f32 / 255.0;

            data[i] = (r - MEAN[0]) / STD[0];
            data[pixels + i] = (g - MEAN[1]) / STD[1];
            data[2 * pixels + i] = (b - MEAN[2]) / STD[2];
        }

        // Tensor [1, 3, 224, 224]
        let tensor = Tensor::from_vec(data, (1usize, 3usize, IMAGE_SIZE, IMAGE_SIZE), &self.device)
            .context("Error creando tensor de imagen")?;

        Ok(tensor)
    }
}
