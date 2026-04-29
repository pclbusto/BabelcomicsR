use anyhow::{Result, bail};
use sqlx::{Row, SqlitePool};

use crate::helpers::comicvine_client::ComicVineClient;

/// Resultado de importar una editorial desde Comic Vine.
pub struct ImportReport {
    pub publisher_name: String,
    pub publisher_local_id: i64,
    pub volumes_inserted: usize,
    pub volumes_skipped: usize,
}

/// Importa una editorial completa desde Comic Vine:
/// 1. Obtiene los detalles del publisher (nombre, descripción, logo, colección de volúmenes).
/// 2. Inserta o actualiza el publisher en la tabla `publishers`.
/// 3. Inserta todos los volúmenes asociados como stubs (local = 1) usando INSERT OR IGNORE
///    para no duplicar los que ya existan.
///
/// Todo el paso 3 se ejecuta dentro de una sola transacción para máxima eficiencia.
pub async fn import_publisher_from_cv(
    pool: &SqlitePool,
    client: &ComicVineClient,
    cv_publisher_id: i64,
) -> Result<ImportReport> {
    // ── 1. Detalle del publisher ──────────────────────────────────────────────
    tracing::info!("Importando publisher CV id={}", cv_publisher_id);
    let details = client
        .get_publisher_details(&cv_publisher_id.to_string())
        .await
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Comic Vine no devolvió datos para publisher id={}. \
                 Revisa los logs de INFO/ERROR para ver la URL y la respuesta exacta.",
                cv_publisher_id
            )
        })?;

    let name = details["name"]
        .as_str()
        .unwrap_or("Editorial desconocida")
        .to_string();

    let description = details["description"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let image_url = details["image"]["medium_url"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    if name.is_empty() {
        bail!(
            "La API devolvió un nombre de editorial vacío para id {}",
            cv_publisher_id
        );
    }

    // ── 2. Upsert del publisher ───────────────────────────────────────────────
    let publisher_local_id = upsert_publisher(
        pool,
        cv_publisher_id,
        &name,
        description.as_deref(),
        image_url.as_deref(),
    )
    .await?;

    // ── 3. Importar volúmenes asociados ───────────────────────────────────────
    let volumes_arr = details["volumes"].as_array().cloned().unwrap_or_default();

    let total = volumes_arr.len();
    let mut inserted = 0usize;

    // Toda la inserción en bloque dentro de una transacción
    let mut tx = pool.begin().await?;

    for vol in &volumes_arr {
        let vol_cv_id = match vol["id"].as_i64() {
            Some(id) => id,
            None => continue,
        };
        let vol_name = vol["name"].as_str().unwrap_or("Sin nombre");
        let api_detail_url = vol["api_detail_url"].as_str().unwrap_or("");

        let affected = sqlx::query(
            r#"INSERT OR IGNORE INTO volumens
                   (nombre, deck, descripcion, url, id_publisher,
                    anio_inicio, cantidad_numeros, id_comicvine, image_url, local)
               VALUES (?, '', '', ?, ?, 0, 0, ?, '', 1)"#,
        )
        .bind(vol_name)
        .bind(api_detail_url)
        .bind(publisher_local_id)
        .bind(vol_cv_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();

        if affected > 0 {
            inserted += 1;
        }
    }

    tx.commit().await?;

    Ok(ImportReport {
        publisher_name: name,
        publisher_local_id,
        volumes_inserted: inserted,
        volumes_skipped: total - inserted,
    })
}

// ── Helpers privados ──────────────────────────────────────────────────────────

/// INSERT o UPDATE del publisher según `id_comicvine`. Devuelve el `id_publisher` local.
async fn upsert_publisher(
    pool: &SqlitePool,
    cv_id: i64,
    nombre: &str,
    descripcion: Option<&str>,
    image_url: Option<&str>,
) -> Result<i64> {
    // Upsert: si ya existe el id_comicvine actualiza los campos de texto.
    sqlx::query(
        r#"INSERT INTO publishers (nombre, descripcion, id_comicvine, image_url)
           VALUES (?, ?, ?, ?)
           ON CONFLICT(id_comicvine) DO UPDATE SET
               nombre      = excluded.nombre,
               descripcion = excluded.descripcion,
               image_url   = excluded.image_url"#,
    )
    .bind(nombre)
    .bind(descripcion)
    .bind(cv_id)
    .bind(image_url)
    .execute(pool)
    .await?;

    // Obtener el id_publisher (ya sea recién insertado o preexistente)
    let row = sqlx::query("SELECT id_publisher FROM publishers WHERE id_comicvine = ?")
        .bind(cv_id)
        .fetch_one(pool)
        .await?;

    Ok(row.get::<i64, _>(0))
}
