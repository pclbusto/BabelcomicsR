use anyhow::Result;
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use std::path::Path;

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

    // Ejecutar migraciones
    sqlx::migrate!("./migrations").run(&pool).await?;

    tracing::info!("Base de datos inicializada: {}", db_path);
    Ok(pool)
}
