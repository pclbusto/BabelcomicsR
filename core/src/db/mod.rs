use anyhow::Result;
use sqlx::{Row, SqlitePool, sqlite::SqlitePoolOptions};
use std::path::Path;

const READING_PROGRESS_V19_ORIGINAL_CHECKSUM: &str = "A15C85003EE1CFF17840540BD05A7A9EBA957434488F322CD8FDAD410EA7B482590E4AFF01B45D2E0E6B7BF4B244F4A6";
const READING_PROGRESS_V19_CURRENT_CHECKSUM: &str = "A5F9B20553BC94C5A4F62A444CAE7CB5755F6CDAEE6972FD0B7B14232D5A29AF41022D880E4446C217E4EDCA10100369";

pub async fn create_pool(db_path: &str) -> Result<SqlitePool> {
    // Crear directorio si no existe
    if let Some(parent) = Path::new(db_path).parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let url = format!("sqlite://{}?mode=rwc", db_path);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await?;

    // Habilitar foreign keys y WAL para mejor rendimiento
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await?;
    sqlx::query("PRAGMA journal_mode = WAL")
        .execute(&pool)
        .await?;

    repair_reading_progress_v19_checksum(&pool).await?;

    // Ejecutar migraciones
    sqlx::migrate!("./migrations").run(&pool).await?;

    tracing::info!("Base de datos inicializada: {}", db_path);
    Ok(pool)
}

/// Compatibilidad con la primera versión de la migración 19, que llegó a
/// ejecutarse localmente antes de que se estabilizara su archivo SQL.
///
/// SQLx rechaza correctamente una migración aplicada cuyo texto cambió. Solo
/// reconciliamos los dos checksums conocidos cuando la tabla conserva
/// exactamente el esquema creado por aquella primera versión. La migración
/// 20 puede ejecutarse normalmente después de esta corrección.
async fn repair_reading_progress_v19_checksum(pool: &SqlitePool) -> Result<()> {
    let migrations_table_exists = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations')",
    )
    .fetch_one(pool)
    .await?
        != 0;

    if !migrations_table_exists {
        return Ok(());
    }

    let has_legacy_checksum = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(\
            SELECT 1 FROM _sqlx_migrations \
            WHERE version = 19 AND success = 1 AND hex(checksum) = ?\
        )",
    )
    .bind(READING_PROGRESS_V19_ORIGINAL_CHECKSUM)
    .fetch_one(pool)
    .await?
        != 0;

    if !has_legacy_checksum {
        return Ok(());
    }

    let columns = sqlx::query("PRAGMA table_info(comicbook_reading_progress)")
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| row.get::<String, _>("name"))
        .collect::<Vec<_>>();
    let expected_columns = [
        "comicbook_id",
        "source_path",
        "source_size_bytes",
        "source_modified_unix_ms",
        "total_pages",
        "last_page_index",
        "visited_pages_json",
        "completed_at",
        "updated_at",
    ];

    if columns != expected_columns {
        return Ok(());
    }

    let updated = sqlx::query(
        "UPDATE _sqlx_migrations \
         SET checksum = CAST(X'A5F9B20553BC94C5A4F62A444CAE7CB5755F6CDAEE6972FD0B7B14232D5A29AF41022D880E4446C217E4EDCA10100369' AS BLOB) \
         WHERE version = 19 AND success = 1 AND hex(checksum) = ?",
    )
    .bind(READING_PROGRESS_V19_ORIGINAL_CHECKSUM)
    .execute(pool)
    .await?;

    if updated.rows_affected() == 1 {
        tracing::warn!(
            from = READING_PROGRESS_V19_ORIGINAL_CHECKSUM,
            to = READING_PROGRESS_V19_CURRENT_CHECKSUM,
            "Se reconcilió el checksum de la migración 19"
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reconciles_known_v19_checksum_when_legacy_schema_matches() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE _sqlx_migrations (\
                version BIGINT PRIMARY KEY, success BOOLEAN NOT NULL, checksum BLOB NOT NULL\
             );\
             INSERT INTO _sqlx_migrations VALUES (19, 1, X'A15C85003EE1CFF17840540BD05A7A9EBA957434488F322CD8FDAD410EA7B482590E4AFF01B45D2E0E6B7BF4B244F4A6');\
             CREATE TABLE comicbook_reading_progress (\
                comicbook_id INTEGER PRIMARY KEY, source_path TEXT NOT NULL,\
                source_size_bytes INTEGER NOT NULL DEFAULT 0,\
                source_modified_unix_ms INTEGER NOT NULL DEFAULT 0,\
                total_pages INTEGER NOT NULL DEFAULT 0, last_page_index INTEGER,\
                visited_pages_json TEXT NOT NULL DEFAULT '[]', completed_at TEXT,\
                updated_at TEXT NOT NULL\
             )",
        )
        .execute(&pool)
        .await
        .unwrap();

        repair_reading_progress_v19_checksum(&pool).await.unwrap();

        let checksum: String =
            sqlx::query_scalar("SELECT hex(checksum) FROM _sqlx_migrations WHERE version = 19")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(checksum, READING_PROGRESS_V19_CURRENT_CHECKSUM);
    }
}
