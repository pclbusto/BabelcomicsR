use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::Utc;
use sqlx::{Row, SqlitePool};

use crate::helpers::reading_progress;
use crate::models::{ReadingProgress, ReadingProgressSource};

pub struct ReadingProgressRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> ReadingProgressRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn get(&self, comicbook_id: i64) -> Result<Option<ReadingProgress>> {
        let row = sqlx::query(
            r#"SELECT source_path, source_size_bytes, source_modified_unix_ms,
                      total_pages, last_page_index, visited_pages_json,
                      completed_at, updated_at
               FROM comicbook_reading_progress
               WHERE comicbook_id = ?"#,
        )
        .bind(comicbook_id)
        .fetch_optional(self.pool)
        .await?;

        row.map(|row| {
            let visited_json: String = row.get("visited_pages_json");
            let mut progress = ReadingProgress {
                schema_version: 1,
                comicbook_id,
                source: ReadingProgressSource {
                    path: row.get("source_path"),
                    size_bytes: i64_to_u64(row.get("source_size_bytes")),
                    modified_unix_ms: i64_to_u64(row.get("source_modified_unix_ms")),
                },
                total_pages: i64_to_usize(row.get("total_pages")),
                last_page_index: row
                    .get::<Option<i64>, _>("last_page_index")
                    .map(i64_to_usize),
                visited_page_indexes: serde_json::from_str(&visited_json)
                    .context("Las páginas visitadas guardadas no son válidas")?,
                completed_at: row.get("completed_at"),
                updated_at: row.get("updated_at"),
            };
            progress.normalize();
            Ok(progress)
        })
        .transpose()
    }

    pub async fn prepare_spool(&self, comicbook_id: i64, comic_path: &Path) -> Result<PathBuf> {
        let current_source = reading_progress::source_for(comic_path);
        let mut progress = self.get(comicbook_id).await?.unwrap_or_default();
        if progress.source != ReadingProgressSource::default()
            && !same_file_version(&progress.source, &current_source)
        {
            progress = ReadingProgress::default();
        }
        progress.comicbook_id = comicbook_id;
        progress.source = current_source;
        progress.normalize();
        let path = reading_progress::spool_path(comicbook_id);
        reading_progress::save_atomically(&path, &progress)?;
        Ok(path)
    }

    pub async fn import_spool(&self, path: &Path) -> Result<ReadingProgress> {
        let mut incoming = reading_progress::load(path)?;
        if incoming.schema_version != 1 {
            bail!(
                "Versión de progreso no soportada: {}",
                incoming.schema_version
            );
        }
        if incoming.comicbook_id <= 0 {
            bail!("El progreso no contiene un comicbook_id válido");
        }
        let comic_exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM comicbooks WHERE id_comicbook = ?)")
                .bind(incoming.comicbook_id)
                .fetch_one(self.pool)
                .await?;
        if !comic_exists {
            bail!("El cómic {} ya no existe", incoming.comicbook_id);
        }

        if let Some(existing) = self.get(incoming.comicbook_id).await?
            && same_file_version(&existing.source, &incoming.source)
        {
            let mut visited = existing
                .visited_page_indexes
                .into_iter()
                .collect::<BTreeSet<_>>();
            visited.extend(incoming.visited_page_indexes);
            incoming.visited_page_indexes = visited.into_iter().collect();
            if incoming.completed_at.is_none() {
                incoming.completed_at = existing.completed_at;
            }
        }
        incoming.normalize();
        if incoming.updated_at.is_empty() {
            incoming.updated_at = Utc::now().to_rfc3339();
        }
        if incoming.is_complete() && incoming.completed_at.is_none() {
            incoming.completed_at = Some(incoming.updated_at.clone());
        }

        let visited_json = serde_json::to_string(&incoming.visited_page_indexes)?;
        sqlx::query(
            r#"INSERT INTO comicbook_reading_progress
                 (comicbook_id, source_path, source_size_bytes, source_modified_unix_ms,
                  total_pages, last_page_index, visited_pages_json, visited_pages_count,
                  completed_at, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(comicbook_id) DO UPDATE SET
                 source_path = excluded.source_path,
                 source_size_bytes = excluded.source_size_bytes,
                 source_modified_unix_ms = excluded.source_modified_unix_ms,
                 total_pages = excluded.total_pages,
                 last_page_index = excluded.last_page_index,
                 visited_pages_json = excluded.visited_pages_json,
                 visited_pages_count = excluded.visited_pages_count,
                 completed_at = excluded.completed_at,
                 updated_at = excluded.updated_at"#,
        )
        .bind(incoming.comicbook_id)
        .bind(&incoming.source.path)
        .bind(u64_to_i64(incoming.source.size_bytes))
        .bind(u64_to_i64(incoming.source.modified_unix_ms))
        .bind(usize_to_i64(incoming.total_pages))
        .bind(incoming.last_page_index.map(usize_to_i64))
        .bind(visited_json)
        .bind(usize_to_i64(incoming.visited_page_indexes.len()))
        .bind(&incoming.completed_at)
        .bind(&incoming.updated_at)
        .execute(self.pool)
        .await?;

        std::fs::remove_file(path).with_context(|| {
            format!(
                "El progreso se guardó, pero no se pudo retirar {}",
                path.display()
            )
        })?;
        Ok(incoming)
    }

    pub async fn import_pending(&self) -> Result<(usize, Vec<String>)> {
        let mut imported = 0;
        let mut errors = Vec::new();
        for path in reading_progress::pending_files()? {
            match self.import_spool(&path).await {
                Ok(_) => imported += 1,
                Err(error) => errors.push(format!("{}: {error}", path.display())),
            }
        }
        Ok((imported, errors))
    }
}

fn same_file_version(left: &ReadingProgressSource, right: &ReadingProgressSource) -> bool {
    left.size_bytes == right.size_bytes
        && left.modified_unix_ms == right.modified_unix_ms
        && (left.size_bytes > 0 || left.path == right.path)
}

fn i64_to_usize(value: i64) -> usize {
    usize::try_from(value.max(0)).unwrap_or(usize::MAX)
}

fn usize_to_i64(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn i64_to_u64(value: i64) -> u64 {
    u64::try_from(value.max(0)).unwrap_or(u64::MAX)
}

fn u64_to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[tokio::test]
    async fn imports_spool_and_removes_it_after_persisting() {
        let unique = format!(
            "babelcomics-reading-progress-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let directory = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&directory).unwrap();
        let database_path = directory.join("test.db");
        let comic_path = directory.join("comic.cbz");
        std::fs::write(&comic_path, b"test comic").unwrap();
        let pool = crate::db::create_pool(database_path.to_str().unwrap())
            .await
            .unwrap();
        let comicbook_id: i64 =
            sqlx::query_scalar("INSERT INTO comicbooks (path) VALUES (?) RETURNING id_comicbook")
                .bind(comic_path.to_string_lossy().as_ref())
                .fetch_one(&pool)
                .await
                .unwrap();

        let spool_path = directory.join("progress.json");
        let progress = ReadingProgress {
            comicbook_id,
            source: reading_progress::source_for(&comic_path),
            total_pages: 4,
            last_page_index: Some(3),
            visited_page_indexes: vec![0, 3],
            updated_at: Utc::now().to_rfc3339(),
            ..ReadingProgress::default()
        };
        reading_progress::save_atomically(&spool_path, &progress).unwrap();

        let repository = ReadingProgressRepository::new(&pool);
        let imported = repository.import_spool(&spool_path).await.unwrap();
        assert_eq!(imported.visited_page_indexes, vec![0, 3]);
        assert_eq!(imported.last_page_index, Some(3));
        assert!(!spool_path.exists());
        let stored = repository.get(comicbook_id).await.unwrap().unwrap();
        assert_eq!(stored.visited_page_indexes, vec![0, 3]);
        assert!(!stored.is_complete());

        let second_spool = directory.join("progress-second-session.json");
        let second_session = ReadingProgress {
            comicbook_id,
            source: reading_progress::source_for(&comic_path),
            total_pages: 4,
            last_page_index: Some(2),
            visited_page_indexes: vec![1, 2],
            updated_at: Utc::now().to_rfc3339(),
            ..ReadingProgress::default()
        };
        reading_progress::save_atomically(&second_spool, &second_session).unwrap();
        let completed = repository.import_spool(&second_spool).await.unwrap();
        assert_eq!(completed.visited_page_indexes, vec![0, 1, 2, 3]);
        assert_eq!(completed.last_page_index, Some(2));
        assert!(completed.is_complete());
        assert!(completed.completed_at.is_some());

        let comics = crate::repositories::ComicbookRepository::new(&pool)
            .get_page(10, 0, false, None)
            .await
            .unwrap();
        assert_eq!(comics.len(), 1);
        assert_eq!(comics[0].reading_total_pages, 4);
        assert_eq!(comics[0].reading_visited_pages, 4);
        assert_eq!(comics[0].reading_last_page_index, Some(2));
        assert!(comics[0].reading_completed_at.is_some());

        pool.close().await;
        let _ = std::fs::remove_dir_all(directory);
    }
}
