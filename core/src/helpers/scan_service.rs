use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use sqlx::SqlitePool;
use std::sync::mpsc::Sender;

use super::clip_embedder;
use crate::repositories::ComicbookRepository;

use super::{
    extractor::extract_cover,
    paths::{comic_thumbnail_path, comicbook_info_thumbnail_path, ensure_thumbnail_dirs},
    scanner::scan_directories,
    thumbnail::{CardSize, thumbnail_exists},
};

/// Flag global para detener todas las tareas pesadas (escaneo, thumbnails) al cerrar la app.
pub static STOP_THREADS: AtomicBool = AtomicBool::new(false);

pub struct ScanResult {
    pub total_found: usize,
    pub new_inserted: u64,
    pub covers_generated: u32,
    pub errors: Vec<String>,
}

/// Escanea los directorios configurados, inserta comics nuevos en la BD
/// y genera thumbnails de portada para los que no los tienen.
pub async fn run_scan(
    pool: &SqlitePool,
    dirs: &[String],
    card_size: CardSize,
) -> Result<ScanResult> {
    let mut result = ScanResult {
        total_found: 0,
        new_inserted: 0,
        covers_generated: 0,
        errors: Vec::new(),
    };

    ensure_thumbnail_dirs()?;

    // 1. Escanear directorios (operación bloqueante — se ejecuta en hilo aparte)
    let dirs_owned = dirs.to_vec();
    let paths = tokio::task::spawn_blocking(move || scan_directories(&dirs_owned)).await?;

    result.total_found = paths.len();
    tracing::info!("Escaneo: {} archivos de comic encontrados", paths.len());

    // 2. Insertar nuevos en la BD
    let repo = ComicbookRepository::new(pool);
    result.new_inserted = repo.insert_batch(&paths).await?;
    tracing::info!("{} comics nuevos insertados", result.new_inserted);

    // 3. Eliminar entradas de archivos que ya no existen en disco
    let deleted = repo.delete_missing_files().await?;
    if deleted > 0 {
        tracing::info!(
            "{} comics eliminados (archivos no encontrados en disco)",
            deleted
        );
    }

    // 4. Generar thumbnails en paralelo con Rayon
    //    Solo para archivos del escaneo actual que aún NO están procesados.
    let scanned: std::collections::HashSet<String> = paths.into_iter().collect();
    let all_comics = repo.get_all_view(false).await?;
    // Filtrar los que realmente necesitan trabajo:
    // - Deben estar en el escaneo actual.
    // - NO deben estar marcados como procesados.
    // - El thumbnail no debe existir (por si acaso).
    let pending: Vec<(i64, String)> = all_comics
        .into_iter()
        .filter(|c| scanned.contains(&c.path))
        // Saltamos si el thumbnail ya existe en disco.
        // Si procesado=1 pero el thumbnail desapareció (y no hay error conocido), reintentamos.
        .filter(|c| !thumbnail_exists(&comic_thumbnail_path(c.id_comicbook, card_size)))
        .filter(|c| c.error_ultimo_escaneo.is_none()) // no reintentar errores conocidos
        .map(|c| (c.id_comicbook, c.path))
        .collect();

    let total_pending = pending.len();
    if total_pending > 0 {
        tracing::info!(
            "Generando thumbnails para {} comics en paralelo...",
            total_pending
        );
        let (generated, errors_batch) = generate_thumbnails_batch(pool, pending, card_size).await?;
        result.covers_generated = generated;
        result.errors = errors_batch;
    }

    tracing::info!(
        "Escaneo completado — {} portadas generadas, {} errores",
        result.covers_generated,
        result.errors.len()
    );

    Ok(result)
}

/// Genera thumbnails para todos los comics que no tienen uno en disco y no han sido procesados.
pub async fn generate_missing_thumbnails(
    pool: &SqlitePool,
    card_size: CardSize,
) -> Result<ScanResult> {
    let mut result = ScanResult {
        total_found: 0,
        new_inserted: 0,
        covers_generated: 0,
        errors: Vec::new(),
    };

    ensure_thumbnail_dirs()?;

    let repo = ComicbookRepository::new(pool);
    let all_comics = repo.get_all_view(false).await?;
    let pending: Vec<(i64, String)> = all_comics
        .into_iter()
        .filter(|c| !thumbnail_exists(&comic_thumbnail_path(c.id_comicbook, card_size)))
        .filter(|c| c.error_ultimo_escaneo.is_none()) // no reintentar errores conocidos
        .map(|c| (c.id_comicbook, c.path))
        .collect();

    result.total_found = pending.len();

    if pending.is_empty() {
        return Ok(result);
    }

    tracing::info!(
        "Thumbnails faltantes detectados: {} — generando en paralelo…",
        pending.len()
    );

    let (generated, errors_batch) = generate_thumbnails_batch(pool, pending, card_size).await?;
    result.covers_generated = generated;
    result.errors = errors_batch;

    tracing::info!(
        "Thumbnails faltantes completados — {} generados, {} errores",
        result.covers_generated,
        result.errors.len()
    );

    Ok(result)
}

// ---------------------------------------------------------------------------
// Embeddings CLIP
// ---------------------------------------------------------------------------

/// Genera embeddings CLIP para todas las portadas ComicVine sin embedding.
/// Compatibilidad: llama a `generate_clip_embeddings` con los parámetros por defecto.
pub async fn generate_missing_clip_embeddings(pool: &SqlitePool) -> Result<(u32, Vec<String>)> {
    generate_clip_embeddings(pool, None, true, None).await
}

#[derive(Clone, Debug)]
pub struct ClipGenerationProgress {
    pub processed: u32,
    pub total: u32,
    pub generated: u32,
    pub errors: u32,
}

/// Genera embeddings CLIP para portadas ComicVine.
///
/// - `volume_id`: si es `Some`, solo procesa las portadas de ese volumen.
/// - `solo_faltantes`: si es `true`, omite las portadas que ya tienen embedding.
pub async fn generate_clip_embeddings(
    pool: &SqlitePool,
    volume_id: Option<i64>,
    solo_faltantes: bool,
    progress_tx: Option<Sender<ClipGenerationProgress>>,
) -> Result<(u32, Vec<String>)> {
    let pending = crate::repositories::ComicbookInfoRepository::new(pool)
        .get_covers_for_clip(volume_id, solo_faltantes)
        .await?;

    if pending.is_empty() {
        return Ok((0, Vec::new()));
    }

    let total = pending.len();
    tracing::info!("Indexando embeddings CLIP para {} portadas…", total);

    // Fase 1: leer archivos en paralelo con rayon (I/O bound)
    let file_data: Vec<(i64, Vec<u8>)> = tokio::task::spawn_blocking(move || {
        use rayon::prelude::*;
        pending
            .into_par_iter()
            .filter_map(
                |(cover_id, ruta_local, url_original, vol_nombre, id_volume)| {
                    if STOP_THREADS.load(std::sync::atomic::Ordering::Relaxed) {
                        return None;
                    }

                    let path = if !ruta_local.is_empty() {
                        std::path::PathBuf::from(&ruta_local)
                    } else {
                        let filename = url_original.split('/').last().unwrap_or("");
                        if filename.is_empty() {
                            return None;
                        }
                        comicbook_info_thumbnail_path(&vol_nombre, id_volume, filename)
                    };

                    if !path.exists() {
                        return None;
                    }

                    match std::fs::read(&path) {
                        Ok(bytes) => {
                            tracing::info!(
                                "Leído cover_id={cover_id} ({} bytes) — {}",
                                bytes.len(),
                                path.display()
                            );
                            Some((cover_id, bytes))
                        }
                        Err(e) => {
                            tracing::warn!(
                                "cover_id={cover_id} — error leyendo {}: {e}",
                                path.display()
                            );
                            None
                        }
                    }
                },
            )
            .collect()
    })
    .await?;

    tracing::info!(
        "Archivos leídos: {}/{}. Iniciando inferencia CLIP…",
        file_data.len(),
        total
    );

    // Fase 2: inferencia secuencial (el modelo CLIP usa todos los cores internamente via BLAS)
    let db_pool = pool.clone();
    let (generated, errors) = tokio::task::spawn_blocking(move || {
        let repo_rt = tokio::runtime::Handle::current();
        let repo = crate::repositories::ComicbookInfoRepository::new(&db_pool);
        let mut generated = 0u32;
        let mut errors = Vec::new();
        let total_files = file_data.len() as u32;

        for (i, (cover_id, bytes)) in file_data.iter().enumerate() {
            if STOP_THREADS.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }

            tracing::info!(
                "[{}/{}] cover_id={cover_id} — generando embedding…",
                i + 1,
                file_data.len()
            );
            match clip_embedder::embed_image(bytes) {
                Ok(emb) => {
                    let blob = clip_embedder::to_bytes(&emb);
                    match repo_rt.block_on(repo.set_cover_clip_embedding(*cover_id, &blob)) {
                        Ok(_) => {
                            generated += 1;
                            tracing::info!(
                                "[{}/{}] cover_id={cover_id} — guardado (total: {generated})",
                                i + 1,
                                file_data.len()
                            );
                        }
                        Err(e) => {
                            tracing::error!("cover_id={cover_id} — error BD: {e}");
                            errors.push(format!("cover {cover_id}: bd: {e}"));
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "[{}/{}] cover_id={cover_id} — error embedding: {e}",
                        i + 1,
                        file_data.len()
                    );
                    errors.push(format!("cover {cover_id}: embed: {e}"));
                }
            }

            if let Some(tx) = &progress_tx {
                let _ = tx.send(ClipGenerationProgress {
                    processed: (i + 1) as u32,
                    total: total_files,
                    generated,
                    errors: errors.len() as u32,
                });
            }
        }
        (generated, errors)
    })
    .await?;
    tracing::info!(
        "CLIP completado — {} indexadas, {} errores",
        generated,
        errors.len()
    );
    if generated > 0 || !errors.is_empty() {
        crate::helpers::suggestion_service::clear_clip_index_cache();
        tracing::info!("CLIP: caché de índice invalidado");
    }
    Ok((generated, errors))
}

/// Versión simplificada sin descarga: solo enlaza ruta_local en BD para portadas
/// cuyo archivo ya existe en disco. No se usa actualmente pero se mantiene para
/// posibles migraciones futuras.
pub async fn relink_covers_from_disk(_pool: &SqlitePool) -> Result<(u32, Vec<String>)> {
    Ok((0, Vec::new()))
}

/// Genera thumbnails en paralelo y registra los errores/éxito en la base de datos.
async fn generate_thumbnails_batch(
    pool: &SqlitePool,
    pending: Vec<(i64, String)>,
    _size: CardSize,
) -> Result<(u32, Vec<String>)> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(i64, Option<String>)>();

    // Rayon para la extracción/redimensión (CPU-bound).
    // Usamos la mitad de los núcleos disponibles para no competir con la UI.
    let rayon_handle = tokio::task::spawn_blocking(move || {
        use rayon::prelude::*;

        let bg_threads = (std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            / 2)
        .max(1);
        let bg_pool = rayon::ThreadPoolBuilder::new()
            .num_threads(bg_threads)
            .build()
            .expect("no se pudo crear threadpool de rayon");

        bg_pool.install(|| {
            pending
                .into_par_iter()
                .map(|(id, path)| {
                    // Si la app está cerrando, abortamos el procesamiento de este lote.
                    if STOP_THREADS.load(Ordering::Relaxed) {
                        return (0u32, None);
                    }

                    // Si la UI ya lo generó prioritariamente, lo saltamos.
                    if thumbnail_exists(&comic_thumbnail_path(id, _size)) {
                        return (0u32, None);
                    }

                    // Generamos SIEMPRE los 3 tamaños de una sola vez
                    match extract_cover(&path)
                        .and_then(|bytes| super::thumbnail::generate_all_thumbnails(&bytes, id))
                    {
                        Ok(()) => {
                            let _ = tx.send((id, None));
                            (1u32, None)
                        }
                        Err(e) => {
                            let msg = e.to_string();
                            let _ = tx.send((id, Some(msg.clone())));
                            (0u32, Some(format!("Error en '{}': {}", path, msg)))
                        }
                    }
                })
                .fold(
                    || (0u32, Vec::<String>::new()),
                    |(ok, mut errs), (n, err)| {
                        if let Some(e) = err {
                            errs.push(e);
                        }
                        (ok + n, errs)
                    },
                )
                .reduce(
                    || (0u32, Vec::new()),
                    |(ok1, mut errs1), (ok2, errs2)| {
                        errs1.extend(errs2);
                        (ok1 + ok2, errs1)
                    },
                )
        })
    });

    // Mientras Rayon trabaja, persistimos resultados en la BD (I/O-bound)
    let db_pool = pool.clone();
    let db_handle = tokio::spawn(async move {
        let repo = ComicbookRepository::new(&db_pool);
        while let Some((id, error)) = rx.recv().await {
            // set_error_ultimo_escaneo ahora también pone procesado = 1
            if let Err(e) = repo.set_error_ultimo_escaneo(id, error.as_deref()).await {
                tracing::error!("Error guardando estado de escaneo para comic {}: {}", id, e);
            }
        }
    });

    let (generated, errors) = rayon_handle.await?;
    db_handle.await?;

    Ok((generated, errors))
}
