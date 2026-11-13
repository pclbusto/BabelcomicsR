use anyhow::Result;
use futures::StreamExt;
use sqlx::{Row, SqlitePool};

use super::clip_embedder;

// ---------------------------------------------------------------------------
// Búsqueda por dHash (perceptual hashing)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SuggestionResult {
    pub id_comicbook_info: i64,
    pub titulo: String,
    pub numero: Option<String>,
    pub id_volume: Option<i64>,
    pub nombre_volume: Option<String>,
    pub distance: u32,
    pub ruta_cover: Option<String>,
    /// URL original de la portada en ComicVine (para construir ruta teórica si ruta_cover es None).
    pub url_original: Option<String>,
}

/// Busca los `max_results` comicbook_info más similares al comic `comic_id`
/// usando el hash perceptual (dHash) almacenado en `comicbooks.embedding`.
///
/// Requiere que los comics catalogados tengan su embedding (dHash) en BD.
pub async fn suggest_for_comic(
    pool: &SqlitePool,
    comic_id: i64,
    max_results: usize,
) -> Result<Vec<SuggestionResult>> {
    // Calcular hash del comic objetivo desde su thumbnail
    let target_hash = tokio::task::spawn_blocking(move || -> Result<String> {
        for size in [
            crate::helpers::thumbnail::CardSize::Medium,
            crate::helpers::thumbnail::CardSize::Large,
            crate::helpers::thumbnail::CardSize::Small,
        ] {
            let thumb = crate::helpers::paths::comic_thumbnail_path(comic_id, size);
            if let Ok(bytes) = std::fs::read(&thumb) {
                if let Some(hash) = crate::helpers::cover_hash::compute_hash(&bytes) {
                    return Ok(hash);
                }
            }
        }
        Err(anyhow::anyhow!(
            "No se encontró thumbnail para el comic {}",
            comic_id
        ))
    })
    .await??;

    let mut rows = sqlx::query(
        r#"SELECT
                cb.embedding,
                ci.id_comicbook_info
           FROM comicbooks cb
           JOIN comicbooks_info ci ON cb.id_comicbook_info = ci.id_comicbook_info
           WHERE cb.en_papelera = 0 AND cb.embedding IS NOT NULL"#,
    )
    .fetch(pool);

    let mut best_by_info: std::collections::HashMap<i64, u32> = std::collections::HashMap::new();

    while let Some(row_res) = rows.next().await {
        let row = row_res?;
        let db_hash: String = row.get(0);

        if let Some(dist) = crate::helpers::cover_hash::distance(&target_hash, &db_hash) {
            let info_id: i64 = row.get(1);
            best_by_info
                .entry(info_id)
                .and_modify(|best| *best = (*best).min(dist))
                .or_insert(dist);
        }
    }

    let mut best_distances: Vec<(i64, u32)> = best_by_info.into_iter().collect();
    best_distances.sort_by_key(|(_, distance)| *distance);
    best_distances.truncate(max_results);

    if best_distances.is_empty() {
        return Ok(Vec::new());
    }

    let info_ids: Vec<i64> = best_distances.iter().map(|(id, _)| *id).collect();
    let distance_map: std::collections::HashMap<i64, u32> = best_distances.into_iter().collect();
    let placeholders = info_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        r#"SELECT
                ci.id_comicbook_info,
                ci.titulo,
                ci.numero,
                ci.id_volume,
                v.nombre,
                (SELECT cic.ruta_local
                 FROM comicbooks_info_covers cic
                 WHERE cic.id_comicbook_info = ci.id_comicbook_info
                 LIMIT 1) as ruta_cover,
                (SELECT cic.url_original
                 FROM comicbooks_info_covers cic
                 WHERE cic.id_comicbook_info = ci.id_comicbook_info
                 LIMIT 1) as url_original
           FROM comicbooks_info ci
           LEFT JOIN volumens v ON ci.id_volume = v.id_volume
           WHERE ci.id_comicbook_info IN ({})"#,
        placeholders
    );

    let mut query = sqlx::query(&sql);
    for id in &info_ids {
        query = query.bind(id);
    }

    let rows = query.fetch_all(pool).await?;
    let mut results: Vec<SuggestionResult> = rows
        .into_iter()
        .filter_map(|row| {
            let info_id: i64 = row.get(0);
            Some(SuggestionResult {
                id_comicbook_info: info_id,
                titulo: row.get(1),
                numero: row.get(2),
                id_volume: row.get(3),
                nombre_volume: row.get(4),
                distance: *distance_map.get(&info_id)?,
                ruta_cover: row.get(5),
                url_original: row.get(6),
            })
        })
        .collect();
    results.sort_by_key(|r| r.distance);

    Ok(results)
}

// ---------------------------------------------------------------------------
// Tipos unificados para la UI
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestionMethod {
    Clip,
    Hash,
}

#[derive(Debug, Clone)]
pub struct UnifiedSuggestion {
    pub id_comicbook_info: i64,
    pub titulo: String,
    pub numero: Option<String>,
    pub id_volume: Option<i64>,
    pub nombre_volume: Option<String>,
    pub similarity: f32, // 0.0 a 1.0
    pub method: SuggestionMethod,
    pub ruta_cover: Option<String>,
    pub url_original: Option<String>,
    pub id_comicvine: Option<i64>,
}

/// Busca los mejores candidatos para un cómic, intentando CLIP primero y
/// cayendo a dHash si no hay resultados CLIP. Garantiza consistencia en toda la app.
pub async fn suggest_best_matches(
    pool: &SqlitePool,
    comic_id: i64,
    max_results: usize,
) -> Result<Vec<UnifiedSuggestion>> {
    // 1. Intentar CLIP (ComicVine index)
    let clip_res = suggest_for_comic_clip(pool, comic_id, max_results).await;

    if let Ok(matches) = clip_res {
        if !matches.is_empty() {
            return Ok(matches
                .into_iter()
                .map(|m| UnifiedSuggestion {
                    id_comicbook_info: m.id_comicbook_info,
                    titulo: m.titulo,
                    numero: m.numero,
                    id_volume: m.id_volume,
                    nombre_volume: m.nombre_volume,
                    similarity: m.similarity,
                    method: SuggestionMethod::Clip,
                    ruta_cover: m.ruta_cover,
                    url_original: m.url_original,
                    id_comicvine: m.id_comicvine,
                })
                .collect());
        }
    }

    // 2. Fallback a dHash (Biblioteca local)
    let hash_res = suggest_for_comic(pool, comic_id, max_results).await;
    match hash_res {
        Ok(matches) => Ok(matches
            .into_iter()
            .map(|m| UnifiedSuggestion {
                id_comicbook_info: m.id_comicbook_info,
                titulo: m.titulo,
                numero: m.numero,
                id_volume: m.id_volume,
                nombre_volume: m.nombre_volume,
                similarity: (64.0 - m.distance as f32) / 64.0,
                method: SuggestionMethod::Hash,
                ruta_cover: m.ruta_cover,
                url_original: m.url_original,
                id_comicvine: None,
            })
            .collect()),
        Err(_) => Ok(Vec::new()),
    }
}

// ---------------------------------------------------------------------------
// Búsqueda visual CLIP
// ---------------------------------------------------------------------------

/// Resultado interno de la búsqueda CLIP (ComicVine).
#[derive(Debug, Clone)]
pub struct ClipSuggestionResult {
    pub id_comicbook_info: i64,
    pub titulo: String,
    pub numero: Option<String>,
    pub id_volume: Option<i64>,
    pub nombre_volume: Option<String>,
    pub similarity: f32,
    pub ruta_cover: Option<String>,
    pub id_comicvine: Option<i64>,
    pub url_original: Option<String>,
}

use std::sync::{Arc, Mutex};

/// (id_comicbook_info, id de la portada, embedding).
type IndexedCover = (i64, i64, Vec<f32>);

lazy_static::lazy_static! {
    static ref CLIP_INDEX_CACHE: Arc<Mutex<Option<Vec<IndexedCover>>>> =
        Arc::new(Mutex::new(None));
}

/// Limpia la caché de embeddings CLIP (llamar tras importar nuevas portadas).
pub fn clear_clip_index_cache() {
    if let Ok(mut cache) = CLIP_INDEX_CACHE.lock() {
        *cache = None;
    }
}

/// Busca los `max_results` comicbook_info más similares a la portada del archivo
/// `comic_id` comparando contra el índice visual de portadas ComicVine.
pub async fn suggest_for_comic_clip(
    pool: &SqlitePool,
    comic_id: i64,
    max_results: usize,
) -> Result<Vec<ClipSuggestionResult>> {
    // 1. Obtener o calcular el embedding del CBZ
    let target_emb = get_or_compute_comic_embedding(pool, comic_id).await?;
    let target_emb = match target_emb {
        Some(e) => e,
        None => {
            anyhow::bail!(
                "No se pudo obtener embedding CLIP para el cómic {}. ¿Tiene portada?",
                comic_id
            );
        }
    };

    // 2. Cargar embeddings indexados (con caché en memoria).
    // El lock se adquiere y suelta antes del await (clone) para que el future sea Send.
    let cached: Option<Vec<IndexedCover>> = CLIP_INDEX_CACHE.lock().unwrap().clone();
    let indexed: Vec<IndexedCover> = if let Some(data) = cached {
        data
    } else {
        let db_indexed = crate::repositories::ComicbookInfoRepository::new(pool)
            .get_all_cover_clip_embeddings()
            .await?;
        let parsed: Vec<IndexedCover> = db_indexed
            .into_iter()
            .filter_map(|(info_id, cover_id, blob)| {
                clip_embedder::from_bytes(&blob).map(|emb| (info_id, cover_id, emb))
            })
            .collect();
        *CLIP_INDEX_CACHE.lock().unwrap() = Some(parsed.clone());
        parsed
    };

    if indexed.is_empty() {
        return Ok(Vec::new());
    }

    // 3. Calcular similitud coseno para cada portada indexada (CPU-bound).
    // Un número puede tener varias portadas (alternativas): se queda con la
    // que mejor coincide y recuerda cuál fue, para mostrar esa misma imagen.
    let results = tokio::task::spawn_blocking(move || -> Vec<(i64, i64, f32)> {
        let mut scored: Vec<(i64, i64, f32)> = indexed
            .into_iter()
            .map(|(info_id, cover_id, emb)| {
                (
                    info_id,
                    cover_id,
                    clip_embedder::cosine_similarity(&target_emb, &emb),
                )
            })
            .collect();

        scored.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

        let mut seen = std::collections::HashSet::new();
        scored.retain(|(id, _, _)| seen.insert(*id));
        scored.truncate(max_results);
        scored
    })
    .await?;

    if results.is_empty() {
        return Ok(Vec::new());
    }

    // 4. Enriquecer con metadata (Query única IN)
    let info_ids: Vec<i64> = results.iter().map(|(id, _, _)| *id).collect();
    let matched: std::collections::HashMap<i64, (i64, f32)> = results
        .into_iter()
        .map(|(info_id, cover_id, similarity)| (info_id, (cover_id, similarity)))
        .collect();

    let placeholders = info_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        r#"SELECT
            ci.id_comicbook_info,
            ci.titulo,
            ci.numero,
            ci.id_volume,
            v.nombre,
            ci.id_comicvine
           FROM comicbooks_info ci
           LEFT JOIN volumens v ON ci.id_volume = v.id_volume
           WHERE ci.id_comicbook_info IN ({})"#,
        placeholders
    );
    let mut query = sqlx::query(&sql);
    for id in &info_ids {
        query = query.bind(id);
    }
    let rows = query.fetch_all(pool).await?;

    // Imagen de la portada que coincidió (puede ser una alternativa).
    let cover_ids: Vec<i64> = matched.values().map(|(cover_id, _)| *cover_id).collect();
    let cover_sql = format!(
        "SELECT id, ruta_local, url_original FROM comicbooks_info_covers WHERE id IN ({})",
        cover_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",")
    );
    let mut cover_query = sqlx::query(&cover_sql);
    for id in &cover_ids {
        cover_query = cover_query.bind(id);
    }
    let covers: std::collections::HashMap<i64, (Option<String>, Option<String>)> = cover_query
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|r| (r.get(0), (r.get(1), r.get(2))))
        .collect();

    let mut enriched: Vec<ClipSuggestionResult> = rows
        .into_iter()
        .map(|r| {
            let id: i64 = r.get(0);
            let (cover_id, similarity) = matched.get(&id).copied().unwrap_or((0, 0.0));
            let (ruta_cover, url_original) = covers.get(&cover_id).cloned().unwrap_or_default();
            ClipSuggestionResult {
                id_comicbook_info: id,
                titulo: r.get(1),
                numero: r.get(2),
                id_volume: r.get(3),
                nombre_volume: r.get(4),
                similarity,
                ruta_cover,
                id_comicvine: r.get(5),
                url_original,
            }
        })
        .collect();

    enriched.sort_by(|a, b| {
        b.similarity
            .partial_cmp(&a.similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(enriched)
}

async fn get_or_compute_comic_embedding(
    pool: &SqlitePool,
    comic_id: i64,
) -> Result<Option<Vec<f32>>> {
    use crate::helpers::thumbnail::CardSize;
    use crate::repositories::ComicbookRepository;

    let repo = ComicbookRepository::new(pool);

    // Camino rápido: embedding ya calculado
    if let Some(blob) = repo.get_clip_embedding(comic_id).await? {
        return Ok(clip_embedder::from_bytes(&blob));
    }

    let comic = match repo.get_by_id(comic_id).await? {
        Some(c) => c,
        None => return Ok(None),
    };

    // Solo intentar el embedding si el modelo ya está cargado en memoria.
    // Evita descargar 340 MB inesperadamente desde HuggingFace Hub, lo que
    // bloquea el proceso al cerrarse (los manejadores atexit esperan la descarga).
    if !clip_embedder::is_model_loaded() {
        return Ok(None);
    }

    let emb_opt = tokio::task::spawn_blocking({
        let path = comic.path.clone();
        move || -> Option<Vec<f32>> {
            // 1. Thumbnail en disco (generado por el escaneo — casi siempre existe)
            let bytes = [CardSize::Large, CardSize::Medium, CardSize::Small]
                .iter()
                .find_map(|&size| {
                    let p = crate::helpers::paths::comic_thumbnail_path(comic_id, size);
                    p.exists().then(|| std::fs::read(&p).ok()).flatten()
                })
                // 2. Solo si no hay thumbnail: abrir el CBZ y extraer la portada
                .or_else(|| crate::helpers::extractor::extract_cover(&path).ok())?;

            clip_embedder::embed_image(&bytes).ok()
        }
    })
    .await?;

    let emb = match emb_opt {
        Some(e) => e,
        None => return Ok(None),
    };

    let blob = clip_embedder::to_bytes(&emb);
    let _ = repo.set_clip_embedding(comic_id, &blob).await;

    Ok(Some(emb))
}
