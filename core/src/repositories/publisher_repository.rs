use anyhow::Result;
use sqlx::SqlitePool;

use crate::models::{NewPublisher, Publisher};

pub struct PublisherRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> PublisherRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn get_all(&self) -> Result<Vec<Publisher>> {
        let rows = sqlx::query_as!(
            Publisher,
            r#"SELECT
                id_publisher as "id_publisher!: i64",
                nombre as "nombre!: String",
                descripcion,
                id_comicvine,
                image_url
               FROM publishers ORDER BY nombre COLLATE NOCASE"#
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn get_by_id(&self, id: i64) -> Result<Option<Publisher>> {
        let row = sqlx::query_as!(
            Publisher,
            r#"SELECT
                id_publisher as "id_publisher!: i64",
                nombre as "nombre!: String",
                descripcion,
                id_comicvine,
                image_url
               FROM publishers WHERE id_publisher = ?"#,
            id
        )
        .fetch_optional(self.pool)
        .await?;
        Ok(row)
    }

    pub async fn get_by_comicvine_id(&self, comicvine_id: i64) -> Result<Option<Publisher>> {
        let row = sqlx::query_as!(
            Publisher,
            r#"SELECT
                id_publisher as "id_publisher!: i64",
                nombre as "nombre!: String",
                descripcion,
                id_comicvine,
                image_url
               FROM publishers WHERE id_comicvine = ?"#,
            comicvine_id
        )
        .fetch_optional(self.pool)
        .await?;
        Ok(row)
    }

    pub async fn search(&self, query: &str) -> Result<Vec<Publisher>> {
        let pattern = format!("%{}%", query);
        let rows = sqlx::query_as!(
            Publisher,
            r#"SELECT
                id_publisher as "id_publisher!: i64",
                nombre as "nombre!: String",
                descripcion,
                id_comicvine,
                image_url
               FROM publishers WHERE nombre LIKE ? ORDER BY nombre COLLATE NOCASE"#,
            pattern
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn insert(&self, new: &NewPublisher) -> Result<i64> {
        let id = sqlx::query!(
            "INSERT INTO publishers (nombre, descripcion, id_comicvine, image_url)
             VALUES (?, ?, ?, ?)",
            new.nombre,
            new.descripcion,
            new.id_comicvine,
            new.image_url
        )
        .execute(self.pool)
        .await?
        .last_insert_rowid();
        Ok(id)
    }

    pub async fn update(&self, publisher: &Publisher) -> Result<()> {
        sqlx::query!(
            "UPDATE publishers SET nombre = ?, descripcion = ?, id_comicvine = ?, image_url = ?
             WHERE id_publisher = ?",
            publisher.nombre,
            publisher.descripcion,
            publisher.id_comicvine,
            publisher.image_url,
            publisher.id_publisher
        )
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete(&self, id: i64) -> Result<()> {
        sqlx::query!("DELETE FROM publishers WHERE id_publisher = ?", id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn count(&self) -> Result<i64> {
        let row = sqlx::query!(r#"SELECT COUNT(*) as "count!: i64" FROM publishers"#)
            .fetch_one(self.pool)
            .await?;
        Ok(row.count)
    }
}
