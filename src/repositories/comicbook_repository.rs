use anyhow::Result;
use sqlx::{SqlitePool, Row};

use crate::models::{Comicbook, ComicFilter, ComicbookView, NewComicbook, parse_search_query};

pub struct ComicbookRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> ComicbookRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn get_by_id(&self, id: i64) -> Result<Option<Comicbook>> {
        let row = sqlx::query_as::<_, Comicbook>(
            r#"SELECT
                id_comicbook,
                path,
                id_comicbook_info,
                calidad,
                en_papelera,
                embedding,
                error_ultimo_escaneo,
                procesado
               FROM comicbooks WHERE id_comicbook = ?"#
        )
        .bind(id)
        .fetch_optional(self.pool)
        .await?;
        Ok(row)
    }

    pub async fn get_by_path(&self, path: &str) -> Result<Option<Comicbook>> {
        let row = sqlx::query_as::<_, Comicbook>(
            r#"SELECT
                id_comicbook,
                path,
                id_comicbook_info,
                calidad,
                en_papelera,
                embedding,
                error_ultimo_escaneo,
                procesado
               FROM comicbooks WHERE path = ?"#
        )
        .bind(path)
        .fetch_optional(self.pool)
        .await?;
        Ok(row)
    }

    /// Página de datos enriquecidos para lazy loading con soporte para filtros
    pub async fn get_page(
        &self,
        limit: i64,
        offset: i64,
        incluir_papelera: bool,
        filter: Option<&ComicFilter>,
    ) -> Result<Vec<ComicbookView>> {
        let clasificado = filter.and_then(|f| f.clasificado);
        let min_q = filter.and_then(|f| f.min_calidad).map(|v| v as i64);

        // Parsear la búsqueda en tokens AND / NOT
        let parsed = filter
            .and_then(|f| f.query.as_deref())
            .map(parse_search_query)
            .unwrap_or_default();

        // Construir fragmento WHERE dinámico para la búsqueda
        let (search_sql, search_binds) = build_search_fragment(&parsed);

        let sql = format!(
            r#"SELECT
                cb.id_comicbook,
                cb.path,
                cb.en_papelera,
                cb.calidad,
                cb.error_ultimo_escaneo,
                cb.procesado,
                ci.titulo,
                ci.numero,
                ci.calificacion,
                v.nombre  AS nombre_volume,
                p.nombre  AS nombre_publisher,
                (SELECT cic.ruta_local
                 FROM comicbooks_info_covers cic
                 WHERE cic.id_comicbook_info = ci.id_comicbook_info
                 LIMIT 1) AS ruta_cover
               FROM comicbooks cb
               LEFT JOIN comicbooks_info ci ON cb.id_comicbook_info = ci.id_comicbook_info
               LEFT JOIN volumens v          ON ci.id_volume = v.id_volume
               LEFT JOIN publishers p       ON v.id_publisher = p.id_publisher
               WHERE (? = 1 OR cb.en_papelera = 0)
                 {search_sql}
                 AND (? IS NULL OR (ci.id_comicbook_info IS NOT NULL) = ?)
                 AND (? IS NULL OR CAST(cb.calidad AS INTEGER) >= ?)
               ORDER BY COALESCE(ci.titulo, cb.path) COLLATE NOCASE
               LIMIT ? OFFSET ?"#
        );

        let mut q = sqlx::query(&sql).bind(incluir_papelera);
        for bind in &search_binds {
            q = q.bind(bind);
        }
        let rows = q
            .bind(clasificado).bind(clasificado)
            .bind(min_q).bind(min_q)
            .bind(limit)
            .bind(offset)
            .fetch_all(self.pool)
            .await?;

        Ok(rows
            .into_iter()
            .map(|r| ComicbookView {
                id_comicbook: r.get(0),
                path: r.get(1),
                en_papelera: r.get(2),
                calidad: r.get(3),
                error_ultimo_escaneo: r.get(4),
                procesado: r.get(5),
                titulo: r.get(6),
                numero: r.get(7),
                calificacion: r.get(8),
                nombre_volume: r.get(9),
                nombre_publisher: r.get(10),
                ruta_cover: r.get(11),
            })
            .collect())
    }

    /// Vista enriquecida de un único comic por ID
    pub async fn get_view_by_id(&self, id: i64) -> Result<Option<ComicbookView>> {
        let row = sqlx::query(
            r#"SELECT
                cb.id_comicbook,
                cb.path,
                cb.en_papelera,
                cb.calidad,
                cb.error_ultimo_escaneo,
                cb.procesado,
                ci.titulo,
                ci.numero,
                ci.calificacion,
                v.nombre  AS nombre_volume,
                p.nombre  AS nombre_publisher,
                (SELECT cic.ruta_local
                 FROM comicbooks_info_covers cic
                 WHERE cic.id_comicbook_info = ci.id_comicbook_info
                 LIMIT 1) AS ruta_cover
               FROM comicbooks cb
               LEFT JOIN comicbooks_info ci ON cb.id_comicbook_info = ci.id_comicbook_info
               LEFT JOIN volumens v          ON ci.id_volume = v.id_volume
               LEFT JOIN publishers p       ON v.id_publisher = p.id_publisher
               WHERE cb.id_comicbook = ?"#,
        )
        .bind(id)
        .fetch_optional(self.pool)
        .await?;

        Ok(row.map(|r| ComicbookView {
            id_comicbook: r.get(0),
            path: r.get(1),
            en_papelera: r.get(2),
            calidad: r.get(3),
            error_ultimo_escaneo: r.get(4),
            procesado: r.get(5),
            titulo: r.get(6),
            numero: r.get(7),
            calificacion: r.get(8),
            nombre_volume: r.get(9),
            nombre_publisher: r.get(10),
            ruta_cover: r.get(11),
        }))
    }

    /// Lista con datos enriquecidos para mostrar en la UI
    pub async fn get_all_view(&self, incluir_papelera: bool) -> Result<Vec<ComicbookView>> {
        let rows = sqlx::query(
            r#"SELECT
                cb.id_comicbook,
                cb.path,
                cb.en_papelera,
                cb.calidad,
                cb.error_ultimo_escaneo,
                cb.procesado,
                ci.titulo,
                ci.numero,
                ci.calificacion,
                v.nombre  AS nombre_volume,
                p.nombre  AS nombre_publisher,
                (SELECT cic.ruta_local
                 FROM comicbooks_info_covers cic
                 WHERE cic.id_comicbook_info = ci.id_comicbook_info
                 LIMIT 1) AS ruta_cover
               FROM comicbooks cb
               LEFT JOIN comicbooks_info ci ON cb.id_comicbook_info = ci.id_comicbook_info
               LEFT JOIN volumens v          ON ci.id_volume = v.id_volume
               LEFT JOIN publishers p       ON v.id_publisher = p.id_publisher
               WHERE (? = 1 OR cb.en_papelera = 0)
               ORDER BY COALESCE(ci.titulo, cb.path) COLLATE NOCASE"#
        )
        .bind(incluir_papelera)
        .fetch_all(self.pool)
        .await?;

        let views = rows
            .into_iter()
            .map(|r| ComicbookView {
                id_comicbook: r.get(0),
                path: r.get(1),
                en_papelera: r.get(2),
                calidad: r.get(3),
                error_ultimo_escaneo: r.get(4),
                procesado: r.get(5),
                titulo: r.get(6),
                numero: r.get(7),
                calificacion: r.get(8),
                nombre_volume: r.get(9),
                nombre_publisher: r.get(10),
                ruta_cover: r.get(11),
            })
            .collect();
        Ok(views)
    }

    pub async fn get_uncatalogued(&self) -> Result<Vec<Comicbook>> {
        let rows = sqlx::query_as::<_, Comicbook>(
            r#"SELECT
                id_comicbook,
                path,
                id_comicbook_info,
                calidad,
                en_papelera,
                embedding,
                error_ultimo_escaneo,
                procesado
               FROM comicbooks
               WHERE id_comicbook_info IS NULL AND en_papelera = 0
               ORDER BY path"#
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn insert(&self, new: &NewComicbook) -> Result<i64> {
        let id = sqlx::query(
            "INSERT OR IGNORE INTO comicbooks (path, id_comicbook_info, calidad)
             VALUES (?, ?, ?)"
        )
        .bind(&new.path)
        .bind(new.id_comicbook_info)
        .bind(&new.calidad)
        .execute(self.pool)
        .await?
        .last_insert_rowid();
        Ok(id)
    }

    /// Inserta múltiples comicbooks en una transacción (para escaneo masivo)
    pub async fn insert_batch(&self, paths: &[String]) -> Result<u64> {
        let mut tx = self.pool.begin().await?;
        let mut inserted = 0u64;

        for path in paths {
            let result = sqlx::query(
                "INSERT OR IGNORE INTO comicbooks (path) VALUES (?)"
            )
            .bind(path)
            .execute(&mut *tx)
            .await?;
            inserted += result.rows_affected();
        }

        tx.commit().await?;
        Ok(inserted)
    }

    pub async fn set_info(&self, id: i64, info_id: Option<i64>) -> Result<()> {
        sqlx::query(
            "UPDATE comicbooks SET id_comicbook_info = ? WHERE id_comicbook = ?"
        )
        .bind(info_id)
        .bind(id)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_papelera(&self, id: i64, en_papelera: bool) -> Result<()> {
        sqlx::query(
            "UPDATE comicbooks SET en_papelera = ? WHERE id_comicbook = ?"
        )
        .bind(en_papelera)
        .bind(id)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_embedding(&self, id: i64, embedding_json: &str) -> Result<()> {
        sqlx::query(
            "UPDATE comicbooks SET embedding = ? WHERE id_comicbook = ?"
        )
        .bind(embedding_json)
        .bind(id)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    /// Guarda el embedding visual CLIP (BLOB de 512 × f32 = 2048 bytes).
    pub async fn set_clip_embedding(&self, id: i64, blob: &[u8]) -> Result<()> {
        sqlx::query(
            "UPDATE comicbooks SET clip_embedding = ? WHERE id_comicbook = ?"
        )
        .bind(blob)
        .bind(id)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    /// Devuelve el embedding CLIP almacenado para un comic, o `None` si no existe.
    pub async fn get_clip_embedding(&self, id: i64) -> Result<Option<Vec<u8>>> {
        let row = sqlx::query(
            "SELECT clip_embedding FROM comicbooks WHERE id_comicbook = ?"
        )
        .bind(id)
        .fetch_optional(self.pool)
        .await?;
        Ok(row.and_then(|r| r.get(0)))
    }


    pub async fn set_error_ultimo_escaneo(&self, id: i64, error: Option<&str>) -> Result<()> {
        sqlx::query(
            "UPDATE comicbooks SET error_ultimo_escaneo = ?, procesado = 1 WHERE id_comicbook = ?"
        )
        .bind(error)
        .bind(id)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_procesado(&self, id: i64, procesado: bool) -> Result<()> {
        sqlx::query(
            "UPDATE comicbooks SET procesado = ? WHERE id_comicbook = ?"
        )
        .bind(procesado)
        .bind(id)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_by_info_id(&self, info_id: i64) -> Result<Vec<Comicbook>> {
        let rows = sqlx::query_as::<_, Comicbook>(
            r#"SELECT
                id_comicbook,
                path,
                id_comicbook_info,
                calidad,
                en_papelera,
                embedding,
                error_ultimo_escaneo,
                procesado
               FROM comicbooks WHERE id_comicbook_info = ?"#
        )
        .bind(info_id)
        .fetch_all(self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn delete(&self, id: i64) -> Result<()> {
        sqlx::query("DELETE FROM comicbooks WHERE id_comicbook = ?")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_missing_files(&self) -> Result<u64> {
        let all = sqlx::query(
            r#"SELECT id_comicbook, path FROM comicbooks"#
        )
        .fetch_all(self.pool)
        .await?;

        let mut tx = self.pool.begin().await?;
        let mut deleted = 0u64;

        for row in all {
            let id: i64 = row.get(0);
            let path: String = row.get(1);
            if !std::path::Path::new(&path).exists() {
                sqlx::query(
                    "DELETE FROM comicbooks WHERE id_comicbook = ?"
                )
                .bind(id)
                .execute(&mut *tx)
                .await?;
                deleted += 1;
            }
        }

        tx.commit().await?;
        Ok(deleted)
    }

    pub async fn count(&self) -> Result<i64> {
        let row = sqlx::query(
            r#"SELECT COUNT(*) FROM comicbooks WHERE en_papelera = 0"#
        )
        .fetch_one(self.pool)
        .await?;
        Ok(row.get(0))
    }

    pub async fn count_uncatalogued(&self) -> Result<i64> {
        let row = sqlx::query(
            r#"SELECT COUNT(*) FROM comicbooks WHERE id_comicbook_info IS NULL AND en_papelera = 0"#
        )
        .fetch_one(self.pool)
        .await?;
        Ok(row.get(0))
    }

    pub async fn count_with_errors(&self) -> Result<i64> {
        let row = sqlx::query(
            r#"SELECT COUNT(*) FROM comicbooks WHERE error_ultimo_escaneo IS NOT NULL AND en_papelera = 0"#
        )
        .fetch_one(self.pool)
        .await?;
        Ok(row.get(0))
    }

    /// Total de comics que coinciden con un filtro (sin paginación).
    pub async fn count_filtered(&self, filter: &ComicFilter) -> Result<i64> {
        let clasificado = filter.clasificado;
        let min_q = filter.min_calidad.map(|v| v as i64);

        let parsed = filter.query.as_deref()
            .map(parse_search_query)
            .unwrap_or_default();

        let (search_sql, search_binds) = build_search_fragment(&parsed);

        let sql = format!(
            r#"SELECT COUNT(*)
               FROM comicbooks cb
               LEFT JOIN comicbooks_info ci ON cb.id_comicbook_info = ci.id_comicbook_info
               LEFT JOIN volumens v          ON ci.id_volume = v.id_volume
               WHERE cb.en_papelera = 0
                 {search_sql}
                 AND (? IS NULL OR (ci.id_comicbook_info IS NOT NULL) = ?)
                 AND (? IS NULL OR CAST(cb.calidad AS INTEGER) >= ?)"#
        );

        let mut q = sqlx::query(&sql);
        for bind in &search_binds {
            q = q.bind(bind);
        }
        let row = q
            .bind(clasificado).bind(clasificado)
            .bind(min_q).bind(min_q)
            .fetch_one(self.pool)
            .await?;
        Ok(row.get(0))
    }

    /// Comics que aún no tienen thumbnail generado (procesado = 0, sin error previo).
    pub async fn count_without_thumbnail(&self) -> Result<i64> {
        let row = sqlx::query(
            r#"SELECT COUNT(*) FROM comicbooks
               WHERE procesado = 0
                 AND en_papelera = 0
                 AND error_ultimo_escaneo IS NULL"#
        )
        .fetch_one(self.pool)
        .await?;
        Ok(row.get(0))
    }
}

// ---------------------------------------------------------------------------
// Helpers de búsqueda
// ---------------------------------------------------------------------------

/// Genera el fragmento SQL WHERE y los valores a bindear para la búsqueda avanzada.
///
/// Los campos buscados son:
/// - `ci.titulo` (título del issue)
/// - `cb.path` (ruta del archivo)
/// - `v.nombre` (nombre de la serie)
///
/// Cada palabra de inclusión genera:
///   `AND (col1 LIKE ? OR col2 LIKE ? OR col3 LIKE ?)`
///
/// Cada palabra de exclusión genera:
///   `AND (col1 NOT LIKE ? AND col2 NOT LIKE ? AND col3 NOT LIKE ?)`
fn build_search_fragment(
    parsed: &crate::models::ParsedQuery,
) -> (String, Vec<String>) {
    if parsed.is_empty() {
        return (String::new(), Vec::new());
    }

    const COLS: &str =
        "COALESCE(ci.titulo, '') LIKE ? OR cb.path LIKE ? OR COALESCE(v.nombre, '') LIKE ?";
    const NOT_COLS: &str =
        "COALESCE(ci.titulo, '') NOT LIKE ? AND cb.path NOT LIKE ? AND COALESCE(v.nombre, '') NOT LIKE ?";

    let mut parts = Vec::new();
    let mut binds: Vec<String> = Vec::new();

    for word in &parsed.must_include {
        let pat = format!("%{}%", word);
        parts.push(format!("AND ({})", COLS));
        binds.extend([pat.clone(), pat.clone(), pat]);
    }

    for word in &parsed.must_exclude {
        let pat = format!("%{}%", word);
        parts.push(format!("AND ({})", NOT_COLS));
        binds.extend([pat.clone(), pat.clone(), pat]);
    }

    (parts.join("\n                 "), binds)
}
