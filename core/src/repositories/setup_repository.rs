use anyhow::Result;
use sqlx::SqlitePool;

use crate::models::{
    NewSetupDirectorio, ReaderDirection, ReaderFit, ReaderGuided, Setup, SetupDirectorio,
};

const DEFAULT_KEY: &str = "default";

pub struct SetupRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> SetupRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn get(&self) -> Result<Setup> {
        let row: Option<Setup> = sqlx::query_as::<_, Setup>(
            r#"SELECT
                setupkey,
                api_key_encrypted,
                modo_oscuro,
                thumbnail_size,
                items_por_pagina,
                num_workers,
                idioma,
                carpeta_thumbnails,
                carpeta_catalogo_normalizado,
                intervalo_api,
                api_url,
                clip_al_arranque,
                clip_al_descargar,
                reader_filter,
                catalog_filter,
                volume_update_filter,
                volume_sort_order,
                reader_path,
                reader_fit,
                reader_direction,
                reader_guided
               FROM setups WHERE setupkey = ?"#,
        )
        .bind(DEFAULT_KEY)
        .fetch_optional(self.pool)
        .await?;

        Ok(row.unwrap_or_default())
    }

    pub async fn save(&self, setup: &Setup) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO setups
                (setupkey, api_key_encrypted, modo_oscuro, thumbnail_size, items_por_pagina, num_workers, idioma, carpeta_thumbnails, carpeta_catalogo_normalizado, intervalo_api, api_url, clip_al_arranque, clip_al_descargar, reader_filter, catalog_filter, volume_update_filter, volume_sort_order, reader_path, reader_fit, reader_direction, reader_guided)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(setupkey) DO UPDATE SET
                api_key_encrypted  = excluded.api_key_encrypted,
                modo_oscuro        = excluded.modo_oscuro,
                thumbnail_size     = excluded.thumbnail_size,
                items_por_pagina   = excluded.items_por_pagina,
                num_workers        = excluded.num_workers,
                idioma             = excluded.idioma,
                carpeta_thumbnails = excluded.carpeta_thumbnails,
                carpeta_catalogo_normalizado = excluded.carpeta_catalogo_normalizado,
                intervalo_api      = excluded.intervalo_api,
                api_url            = excluded.api_url,
                clip_al_arranque   = excluded.clip_al_arranque,
                clip_al_descargar  = excluded.clip_al_descargar,
                reader_filter      = excluded.reader_filter,
                catalog_filter     = excluded.catalog_filter,
                volume_update_filter = excluded.volume_update_filter,
                volume_sort_order  = excluded.volume_sort_order,
                reader_path        = excluded.reader_path,
                reader_fit         = excluded.reader_fit,
                reader_direction   = excluded.reader_direction,
                reader_guided      = excluded.reader_guided"#,
        )
        .bind(&setup.setupkey)
        .bind(&setup.api_key_encrypted)
        .bind(setup.modo_oscuro)
        .bind(setup.thumbnail_size)
        .bind(setup.items_por_pagina)
        .bind(setup.num_workers)
        .bind(&setup.idioma)
        .bind(&setup.carpeta_thumbnails)
        .bind(&setup.carpeta_catalogo_normalizado)
        .bind(setup.intervalo_api)
        .bind(&setup.api_url)
        .bind(setup.clip_al_arranque)
        .bind(setup.clip_al_descargar)
        .bind(setup.reader_filter)
        .bind(&setup.catalog_filter)
        .bind(&setup.volume_update_filter)
        .bind(&setup.volume_sort_order)
        .bind(&setup.reader_path)
        .bind(&setup.reader_fit)
        .bind(&setup.reader_direction)
        .bind(&setup.reader_guided)
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

    pub async fn set_reader_filter(&self, filter: i64) -> Result<()> {
        sqlx::query!(
            "UPDATE setups SET reader_filter = ? WHERE setupkey = ?",
            filter,
            DEFAULT_KEY
        )
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_clip_al_arranque(&self, enabled: bool) -> Result<()> {
        sqlx::query("UPDATE setups SET clip_al_arranque = ? WHERE setupkey = ?")
            .bind(enabled)
            .bind(DEFAULT_KEY)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_clip_al_descargar(&self, enabled: bool) -> Result<()> {
        sqlx::query("UPDATE setups SET clip_al_descargar = ? WHERE setupkey = ?")
            .bind(enabled)
            .bind(DEFAULT_KEY)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_reader_path(&self, path: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE setups SET reader_path = ? WHERE setupkey = ?")
            .bind(path)
            .bind(DEFAULT_KEY)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_reader_fit(&self, fit: ReaderFit) -> Result<()> {
        sqlx::query("UPDATE setups SET reader_fit = ? WHERE setupkey = ?")
            .bind(fit.as_str())
            .bind(DEFAULT_KEY)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_reader_direction(&self, direction: ReaderDirection) -> Result<()> {
        sqlx::query("UPDATE setups SET reader_direction = ? WHERE setupkey = ?")
            .bind(direction.as_str())
            .bind(DEFAULT_KEY)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_reader_guided(&self, guided: ReaderGuided) -> Result<()> {
        sqlx::query("UPDATE setups SET reader_guided = ? WHERE setupkey = ?")
            .bind(guided.as_str())
            .bind(DEFAULT_KEY)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_library_filters(
        &self,
        catalog_filter: &str,
        volume_update_filter: &str,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE setups SET catalog_filter = ?, volume_update_filter = ? WHERE setupkey = ?",
        )
        .bind(catalog_filter)
        .bind(volume_update_filter)
        .bind(DEFAULT_KEY)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_volume_sort_order(&self, sort_order: &str) -> Result<()> {
        sqlx::query("UPDATE setups SET volume_sort_order = ? WHERE setupkey = ?")
            .bind(sort_order)
            .bind(DEFAULT_KEY)
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

    pub async fn set_carpeta_catalogo_normalizado(&self, path: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE setups SET carpeta_catalogo_normalizado = ? WHERE setupkey = ?")
            .bind(path)
            .bind(DEFAULT_KEY)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_scan_extensions(&self) -> Result<Vec<String>> {
        use sqlx::Row;

        let rows = sqlx::query(
            "SELECT extension FROM setup_scan_extensions WHERE setup_key = ? ORDER BY extension",
        )
        .bind(DEFAULT_KEY)
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(|row| row.get(0)).collect())
    }

    pub async fn set_scan_extension(&self, extension: &str, enabled: bool) -> Result<()> {
        if enabled {
            sqlx::query(
                "INSERT OR IGNORE INTO setup_scan_extensions (setup_key, extension) VALUES (?, ?)",
            )
            .bind(DEFAULT_KEY)
            .bind(extension.to_ascii_lowercase())
            .execute(self.pool)
            .await?;
        } else {
            sqlx::query(
                "DELETE FROM setup_scan_extensions WHERE setup_key = ? AND extension = ? COLLATE NOCASE",
            )
            .bind(DEFAULT_KEY)
            .bind(extension)
            .execute(self.pool)
            .await?;
        }
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
