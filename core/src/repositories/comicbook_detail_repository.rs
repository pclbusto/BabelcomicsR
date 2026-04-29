use anyhow::Result;
use sqlx::SqlitePool;

use crate::models::{ComicbookDetail, NewComicbookDetail};

pub struct ComicbookDetailRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> ComicbookDetailRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Todas las páginas de un comic ordenadas por indicePagina
    pub async fn get_by_comicbook(&self, comicbook_id: i64) -> Result<Vec<ComicbookDetail>> {
        let rows = sqlx::query_as::<_, ComicbookDetail>(
            r#"SELECT id_detail, comicbook_id, indicePagina, ordenPagina, tipoPagina, nombre_pagina
               FROM comicbooks_detail
               WHERE comicbook_id = ?
               ORDER BY indicePagina"#,
        )
        .bind(comicbook_id)
        .fetch_all(self.pool)
        .await?;
        Ok(rows)
    }

    /// La portada (tipoPagina = 1 = FrontCover), o la primera página si no hay ninguna marcada
    pub async fn get_cover(&self, comicbook_id: i64) -> Result<Option<ComicbookDetail>> {
        let row = sqlx::query_as::<_, ComicbookDetail>(
            r#"SELECT id_detail, comicbook_id, indicePagina, ordenPagina, tipoPagina, nombre_pagina
               FROM comicbooks_detail
               WHERE comicbook_id = ?
               ORDER BY (tipoPagina = 1) DESC, indicePagina ASC
               LIMIT 1"#,
        )
        .bind(comicbook_id)
        .fetch_optional(self.pool)
        .await?;
        Ok(row)
    }

    /// Inserta o reemplaza todas las páginas de un comic (usado al escanear)
    pub async fn upsert_all(&self, pages: &[NewComicbookDetail]) -> Result<()> {
        if pages.is_empty() {
            return Ok(());
        }
        for p in pages {
            sqlx::query(
                r#"INSERT INTO comicbooks_detail
                    (comicbook_id, indicePagina, ordenPagina, tipoPagina, nombre_pagina)
                   VALUES (?, ?, ?, ?, ?)
                   ON CONFLICT(comicbook_id, indicePagina) DO UPDATE SET
                    ordenPagina   = excluded.ordenPagina,
                    tipoPagina    = excluded.tipoPagina,
                    nombre_pagina = excluded.nombre_pagina"#,
            )
            .bind(p.comicbook_id)
            .bind(p.indice_pagina)
            .bind(p.orden_pagina)
            .bind(p.tipo_pagina.to_db())
            .bind(&p.nombre_pagina)
            .execute(self.pool)
            .await?;
        }
        Ok(())
    }

    /// Marca una página como portada (FrontCover) y quita la marca de las demás
    pub async fn set_as_cover(&self, comicbook_id: i64, indice_pagina: i64) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        // Desmarcar portada anterior
        sqlx::query(
            "UPDATE comicbooks_detail SET tipoPagina = 0 WHERE comicbook_id = ? AND tipoPagina = 1",
        )
        .bind(comicbook_id)
        .execute(&mut *tx)
        .await?;

        // Marcar la nueva
        sqlx::query(
            "UPDATE comicbooks_detail SET tipoPagina = 1 WHERE comicbook_id = ? AND indicePagina = ?",
        )
        .bind(comicbook_id)
        .bind(indice_pagina)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    /// Elimina todos los registros de un comic (para re-escanear)
    pub async fn delete_by_comicbook(&self, comicbook_id: i64) -> Result<()> {
        sqlx::query("DELETE FROM comicbooks_detail WHERE comicbook_id = ?")
            .bind(comicbook_id)
            .execute(self.pool)
            .await?;
        Ok(())
    }
}
