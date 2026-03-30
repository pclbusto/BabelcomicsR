use anyhow::Result;
use sqlx::SqlitePool;

use crate::models::{NewSetupDirectorio, Setup, SetupDirectorio};

const DEFAULT_KEY: &str = "default";

pub struct SetupRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> SetupRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn get(&self) -> Result<Setup> {
        let row: Option<Setup> = sqlx::query_as!(
            Setup,
            r#"SELECT
                setupkey as "setupkey!: String",
                api_key_encrypted,
                modo_oscuro as "modo_oscuro!: bool",
                thumbnail_size as "thumbnail_size!: i64",
                items_por_pagina as "items_por_pagina!: i64",
                num_workers as "num_workers!: i64",
                idioma,
                carpeta_thumbnails,
                intervalo_api as "intervalo_api!: f64",
                api_url
               FROM setups WHERE setupkey = ?"#,
            DEFAULT_KEY
        )
        .fetch_optional(self.pool)
        .await?;

        Ok(row.unwrap_or_default())
    }

    pub async fn save(&self, setup: &Setup) -> Result<()> {
        sqlx::query!(
            r#"INSERT INTO setups
                (setupkey, api_key_encrypted, modo_oscuro, thumbnail_size, items_por_pagina, num_workers, idioma, carpeta_thumbnails, intervalo_api, api_url)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(setupkey) DO UPDATE SET
                api_key_encrypted  = excluded.api_key_encrypted,
                modo_oscuro        = excluded.modo_oscuro,
                thumbnail_size     = excluded.thumbnail_size,
                items_por_pagina   = excluded.items_por_pagina,
                num_workers        = excluded.num_workers,
                idioma             = excluded.idioma,
                carpeta_thumbnails = excluded.carpeta_thumbnails,
                intervalo_api      = excluded.intervalo_api,
                api_url            = excluded.api_url"#,
            setup.setupkey,
            setup.api_key_encrypted,
            setup.modo_oscuro,
            setup.thumbnail_size,
            setup.items_por_pagina,
            setup.num_workers,
            setup.idioma,
            setup.carpeta_thumbnails,
            setup.intervalo_api,
            setup.api_url
        )
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_api_key(&self, encrypted_key: Option<&str>) -> Result<()> {
        sqlx::query!(
            "UPDATE setups SET api_key_encrypted = ? WHERE setupkey = ?",
            encrypted_key,
            DEFAULT_KEY
        )
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_card_size(&self, size: i64) -> Result<()> {
        sqlx::query!(
            "UPDATE setups SET thumbnail_size = ? WHERE setupkey = ?",
            size,
            DEFAULT_KEY
        )
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_carpeta_thumbnails(&self, path: Option<&str>) -> Result<()> {
        sqlx::query!(
            "UPDATE setups SET carpeta_thumbnails = ? WHERE setupkey = ?",
            path,
            DEFAULT_KEY
        )
        .execute(self.pool)
        .await?;
        Ok(())
    }

    // --- Directorios ---

    pub async fn get_directorios(&self) -> Result<Vec<SetupDirectorio>> {
        let rows = sqlx::query_as!(
            SetupDirectorio,
            r#"SELECT
                id as "id!: i64",
                path as "path!: String",
                setup_key as "setup_key!: String"
               FROM setup_directorios WHERE setup_key = ? ORDER BY path"#,
            DEFAULT_KEY
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn add_directorio(&self, path: &str) -> Result<i64> {
        let new = NewSetupDirectorio {
            path: path.to_string(),
            setup_key: DEFAULT_KEY.to_string(),
        };
        let id = sqlx::query!(
            "INSERT OR IGNORE INTO setup_directorios (path, setup_key) VALUES (?, ?)",
            new.path,
            new.setup_key
        )
        .execute(self.pool)
        .await?
        .last_insert_rowid();
        Ok(id)
    }

    pub async fn remove_directorio(&self, id: i64) -> Result<()> {
        sqlx::query!("DELETE FROM setup_directorios WHERE id = ?", id)
            .execute(self.pool)
            .await?;
        Ok(())
    }
}
