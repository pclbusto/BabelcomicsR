use anyhow::Result;
use sqlx::SqlitePool;

use crate::models::{ComicbookInfo, ComicbookInfoCover, NewComicbookInfo, NewComicbookInfoCover};

pub struct ComicbookInfoRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> ComicbookInfoRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn get_view_by_volume(
        &self,
        volume_id: i64,
    ) -> Result<Vec<crate::models::ComicbookInfoView>> {
        self.get_view_by_volume_page(volume_id, i64::MAX, 0, None, false)
            .await
    }

    pub async fn get_view_by_volume_page(
        &self,
        volume_id: i64,
        limit: i64,
        offset: i64,
        query: Option<&str>,
        solo_poseidos: bool,
    ) -> Result<Vec<crate::models::ComicbookInfoView>> {
        let pattern = query.map(|q| format!("%{}%", q));
        let sql = format!(
            r#"SELECT
                ci.id_comicbook_info, ci.titulo, ci.id_volume, ci.numero, ci.resumen,
                ci.calificacion, ci.id_comicvine, ci.url_api_detalle, ci.fue_actualizado_api,
                (SELECT COUNT(*) FROM comicbooks cb WHERE cb.id_comicbook_info = ci.id_comicbook_info) as physical_count,
                (SELECT cic.ruta_local FROM comicbooks_info_covers cic WHERE cic.id_comicbook_info = ci.id_comicbook_info LIMIT 1) as ruta_cover,
                (SELECT cb.id_comicbook FROM comicbooks cb WHERE cb.id_comicbook_info = ci.id_comicbook_info LIMIT 1) as id_comicbook,
                (SELECT cic.url_original FROM comicbooks_info_covers cic WHERE cic.id_comicbook_info = ci.id_comicbook_info LIMIT 1) as url_original
               FROM comicbooks_info ci
               WHERE ci.id_volume = ?
               {}
               {}
               ORDER BY CAST(ci.numero AS REAL), ci.numero
               LIMIT ? OFFSET ?"#,
            if pattern.is_some() {
                "AND (ci.titulo LIKE ? OR ci.numero LIKE ?)"
            } else {
                ""
            },
            if solo_poseidos {
                "AND (SELECT COUNT(*) FROM comicbooks cb WHERE cb.id_comicbook_info = ci.id_comicbook_info) > 0"
            } else {
                ""
            },
        );

        let mut q = sqlx::query(&sql).bind(volume_id);
        if let Some(ref p) = pattern {
            q = q.bind(p).bind(p);
        }
        let rows = q.bind(limit).bind(offset).fetch_all(self.pool).await?;

        use crate::models::{ComicbookInfo, ComicbookInfoView};
        use sqlx::Row;

        let views = rows
            .into_iter()
            .map(|r| {
                let info = ComicbookInfo {
                    id_comicbook_info: r.get(0),
                    titulo: r.get(1),
                    id_volume: r.get(2),
                    numero: r.get(3),
                    resumen: r.get(4),
                    calificacion: r.get(5),
                    id_comicvine: r.get(6),
                    url_api_detalle: r.get(7),
                    fue_actualizado_api: r.get(8),
                };
                ComicbookInfoView {
                    info,
                    physical_count: r.get(9),
                    ruta_cover: r.get(10),
                    id_comicbook: r.get(11),
                    url_original: r.get(12),
                }
            })
            .collect();

        Ok(views)
    }

    pub async fn get_by_id(&self, id: i64) -> Result<Option<ComicbookInfo>> {
        let row = sqlx::query_as!(
            ComicbookInfo,
            r#"SELECT
                id_comicbook_info as "id_comicbook_info!: i64",
                titulo as "titulo!: String",
                id_volume,
                numero,
                resumen,
                calificacion,
                id_comicvine,
                url_api_detalle,
                fue_actualizado_api as "fue_actualizado_api!: bool"
               FROM comicbooks_info WHERE id_comicbook_info = ?"#,
            id
        )
        .fetch_optional(self.pool)
        .await?;
        Ok(row)
    }

    pub async fn get_by_volume(&self, volume_id: i64) -> Result<Vec<ComicbookInfo>> {
        let rows = sqlx::query_as!(
            ComicbookInfo,
            r#"SELECT
                id_comicbook_info as "id_comicbook_info!: i64",
                titulo as "titulo!: String",
                id_volume,
                numero,
                resumen,
                calificacion,
                id_comicvine,
                url_api_detalle,
                fue_actualizado_api as "fue_actualizado_api!: bool"
               FROM comicbooks_info WHERE id_volume = ?
               ORDER BY CAST(numero AS REAL)"#,
            volume_id
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn get_by_comicvine_id(&self, comicvine_id: i64) -> Result<Option<ComicbookInfo>> {
        let row = sqlx::query_as!(
            ComicbookInfo,
            r#"SELECT
                id_comicbook_info as "id_comicbook_info!: i64",
                titulo as "titulo!: String",
                id_volume,
                numero,
                resumen,
                calificacion,
                id_comicvine,
                url_api_detalle,
                fue_actualizado_api as "fue_actualizado_api!: bool"
               FROM comicbooks_info WHERE id_comicvine = ?"#,
            comicvine_id
        )
        .fetch_optional(self.pool)
        .await?;
        Ok(row)
    }

    pub async fn search(&self, query: &str) -> Result<Vec<ComicbookInfo>> {
        let pattern = format!("%{}%", query);
        let rows = sqlx::query_as!(
            ComicbookInfo,
            r#"SELECT
                id_comicbook_info as "id_comicbook_info!: i64",
                titulo as "titulo!: String",
                id_volume,
                numero,
                resumen,
                calificacion,
                id_comicvine,
                url_api_detalle,
                fue_actualizado_api as "fue_actualizado_api!: bool"
               FROM comicbooks_info WHERE titulo LIKE ?
               ORDER BY titulo COLLATE NOCASE"#,
            pattern
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn insert(&self, new: &NewComicbookInfo) -> Result<i64> {
        let id = sqlx::query!(
            "INSERT INTO comicbooks_info
                (titulo, id_volume, numero, resumen, calificacion, id_comicvine, url_api_detalle)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            new.titulo,
            new.id_volume,
            new.numero,
            new.resumen,
            new.calificacion,
            new.id_comicvine,
            new.url_api_detalle
        )
        .execute(self.pool)
        .await?
        .last_insert_rowid();
        Ok(id)
    }

    pub async fn update(&self, info: &ComicbookInfo) -> Result<()> {
        sqlx::query!(
            "UPDATE comicbooks_info
             SET titulo = ?, id_volume = ?, numero = ?, resumen = ?,
                 calificacion = ?, id_comicvine = ?, url_api_detalle = ?, fue_actualizado_api = ?
             WHERE id_comicbook_info = ?",
            info.titulo,
            info.id_volume,
            info.numero,
            info.resumen,
            info.calificacion,
            info.id_comicvine,
            info.url_api_detalle,
            info.fue_actualizado_api,
            info.id_comicbook_info
        )
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete(&self, id: i64) -> Result<()> {
        sqlx::query!(
            "DELETE FROM comicbooks_info WHERE id_comicbook_info = ?",
            id
        )
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn count(&self) -> Result<i64> {
        let row = sqlx::query!(r#"SELECT COUNT(*) as "count!: i64" FROM comicbooks_info"#)
            .fetch_one(self.pool)
            .await?;
        Ok(row.count)
    }

    // --- Covers ---

    pub async fn get_covers(&self, info_id: i64) -> Result<Vec<ComicbookInfoCover>> {
        let rows = sqlx::query_as!(
            ComicbookInfoCover,
            r#"SELECT
                id as "id!: i64",
                id_comicbook_info as "id_comicbook_info!: i64",
                url_original as "url_original!: String",
                ruta_local
               FROM comicbooks_info_covers WHERE id_comicbook_info = ?"#,
            info_id
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn insert_cover(&self, cover: &NewComicbookInfoCover) -> Result<i64> {
        let id = sqlx::query!(
            "INSERT INTO comicbooks_info_covers (id_comicbook_info, url_original, ruta_local)
             VALUES (?, ?, ?)",
            cover.id_comicbook_info,
            cover.url_original,
            cover.ruta_local
        )
        .execute(self.pool)
        .await?
        .last_insert_rowid();
        Ok(id)
    }

    pub async fn set_cover_local_path(&self, cover_id: i64, ruta_local: &str) -> Result<()> {
        sqlx::query!(
            "UPDATE comicbooks_info_covers SET ruta_local = ? WHERE id = ?",
            ruta_local,
            cover_id
        )
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_covers(&self, info_id: i64) -> Result<()> {
        sqlx::query!(
            "DELETE FROM comicbooks_info_covers WHERE id_comicbook_info = ?",
            info_id
        )
        .execute(self.pool)
        .await?;
        Ok(())
    }

    // --- Embeddings CLIP ---

    pub async fn set_cover_clip_embedding(&self, cover_id: i64, blob: &[u8]) -> Result<()> {
        sqlx::query("UPDATE comicbooks_info_covers SET clip_embedding = ? WHERE id = ?")
            .bind(blob)
            .bind(cover_id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    /// (cover_id, ruta_local_or_empty, url_original, vol_nombre, id_volume) para todas las portadas sin embedding CLIP.
    /// Si ruta_local está seteada se usa directamente; si no, se reconstruye el path desde url+vol.
    pub async fn get_covers_without_clip_embedding(
        &self,
    ) -> Result<Vec<(i64, String, String, String, i64)>> {
        self.get_covers_for_clip(None, true).await
    }

    /// Devuelve portadas para indexación CLIP.
    ///
    /// - `volume_id`: si es `Some`, filtra solo las portadas del volumen dado.
    /// - `solo_faltantes`: si es `true`, devuelve solo las que no tienen embedding aún.
    pub async fn get_covers_for_clip(
        &self,
        volume_id: Option<i64>,
        solo_faltantes: bool,
    ) -> Result<Vec<(i64, String, String, String, i64)>> {
        use sqlx::Row;

        let sql = format!(
            r#"SELECT cic.id,
                      COALESCE(cic.ruta_local, ''),
                      COALESCE(cic.url_original, ''),
                      COALESCE(v.nombre, ''),
                      COALESCE(ci.id_volume, 0)
               FROM comicbooks_info_covers cic
               LEFT JOIN comicbooks_info ci ON ci.id_comicbook_info = cic.id_comicbook_info
               LEFT JOIN volumens v ON v.id_volume = ci.id_volume
               WHERE {}{}
               ORDER BY cic.id"#,
            if solo_faltantes {
                "cic.clip_embedding IS NULL"
            } else {
                "1=1"
            },
            if volume_id.is_some() {
                " AND ci.id_volume = ?"
            } else {
                ""
            },
        );

        let mut q = sqlx::query(&sql);
        if let Some(vid) = volume_id {
            q = q.bind(vid);
        }

        let rows = q.fetch_all(self.pool).await?;
        Ok(rows
            .into_iter()
            .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3), r.get(4)))
            .collect())
    }

    /// (id_comicbook_info, clip_embedding) de todas las portadas indexadas.
    pub async fn get_all_cover_clip_embeddings(&self) -> Result<Vec<(i64, Vec<u8>)>> {
        use sqlx::Row;
        let rows = sqlx::query(
            r#"SELECT id_comicbook_info, clip_embedding
               FROM comicbooks_info_covers
               WHERE clip_embedding IS NOT NULL
               ORDER BY id_comicbook_info, id"#,
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|r| {
                let blob: Option<Vec<u8>> = r.get(1);
                blob.map(|b| (r.get(0), b))
            })
            .collect())
    }

    /// (total, con_archivo_local, ya_indexadas, pendientes)
    pub async fn count_clip_index_stats(
        &self,
        volume_id: Option<i64>,
    ) -> Result<(i64, i64, i64, i64)> {
        use sqlx::Row;
        let sql = format!(
            r#"SELECT
                COUNT(*),
                COUNT(CASE WHEN ruta_local IS NOT NULL THEN 1 END),
                COUNT(CASE WHEN clip_embedding IS NOT NULL THEN 1 END),
                COUNT(CASE WHEN ruta_local IS NOT NULL AND clip_embedding IS NULL THEN 1 END)
               FROM comicbooks_info_covers cic
               LEFT JOIN comicbooks_info ci ON ci.id_comicbook_info = cic.id_comicbook_info
               {}"#,
            if volume_id.is_some() {
                "WHERE ci.id_volume = ?"
            } else {
                ""
            },
        );

        let mut q = sqlx::query(&sql);
        if let Some(vid) = volume_id {
            q = q.bind(vid);
        }

        let row = q.fetch_one(self.pool).await?;
        Ok((row.get(0), row.get(1), row.get(2), row.get(3)))
    }
}
