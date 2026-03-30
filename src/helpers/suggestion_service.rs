use anyhow::Result;
use sqlx::{Row, SqlitePool};
use futures::StreamExt; // Necesario para procesar el stream

#[derive(Debug, Clone)]
pub struct SuggestionResult {
    pub id_comicbook_info: i64,
    pub titulo: String,
    pub numero: Option<String>,
    pub id_volume: Option<i64>,
    pub nombre_volume: Option<String>,
    pub distance: u32,
    pub ruta_cover: Option<String>,
}

pub async fn suggest_for_comic(
    pool: &SqlitePool,
    comic_id: i64,
    max_results: usize,
) -> Result<Vec<SuggestionResult>> {
    // 1. Calcular hash del comic objetivo (esto es rápido)
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
        Err(anyhow::anyhow!("No se encontró thumbnail para el comic {}", comic_id))
    })
    .await??;

    // 2. Procesar la base de datos como un STREAM (uno a uno)
    let mut rows = sqlx::query(
        r#"SELECT
                cb.embedding,
                ci.id_comicbook_info,
                ci.titulo,
                ci.numero,
                ci.id_volume,
                v.nombre,
                (SELECT cic.ruta_local
                 FROM comicbooks_info_covers cic
                 WHERE cic.id_comicbook_info = ci.id_comicbook_info
                 LIMIT 1) as ruta_cover
           FROM comicbooks cb
           JOIN comicbooks_info ci ON cb.id_comicbook_info = ci.id_comicbook_info
           LEFT JOIN volumens v ON ci.id_volume = v.id_volume
           WHERE cb.en_papelera = 0 AND cb.embedding IS NOT NULL"#,
    )
    .fetch(pool); // <-- fetch en lugar de fetch_all

    let mut best_results: Vec<SuggestionResult> = Vec::with_capacity(max_results + 1);
    let mut seen_infos = std::collections::HashSet::new();

    while let Some(row_res) = rows.next().await {
        let row = row_res?;
        let db_hash: String = row.get(0);
        
        // Calcular distancia inmediatamente
        if let Some(dist) = crate::helpers::cover_hash::distance(&target_hash, &db_hash) {
            // Umbral de corte: Si la distancia es enorme, ni nos molestamos en clonar strings
            if dist > 35 && best_results.len() >= max_results {
                continue;
            }

            let info_id: i64 = row.get(1);
            if seen_infos.contains(&info_id) {
                continue;
            }

            // Solo si es un candidato potencial, extraemos el resto de datos
            let res = SuggestionResult {
                id_comicbook_info: info_id,
                titulo: row.get(2),
                numero: row.get(3),
                id_volume: row.get(4),
                nombre_volume: row.get(5),
                distance: dist,
                ruta_cover: row.get(6),
            };

            best_results.push(res);
            best_results.sort_by_key(|r| r.distance);
            
            // Mantener solo los mejores y limpiar el set de vistos
            if best_results.len() > max_results {
                best_results.pop();
            }
            
            // Re-escudriñar el set de vistos para que coincida con best_results
            seen_infos.clear();
            for r in &best_results {
                seen_infos.insert(r.id_comicbook_info);
            }
        }
    }

    Ok(best_results)
}
