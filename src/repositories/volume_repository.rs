use anyhow::Result;
use sqlx::{SqlitePool, Row};

use crate::models::{Volume, NewVolume, parse_search_query};

#[derive(Clone, Copy, Default, PartialEq)]
pub enum VolumeSortOrder {
    #[default]
    NombreAsc,
    NombreDesc,
    AnioAsc,
    AnioDesc,
    IssuesAsc,
    IssuesDesc,
}

impl VolumeSortOrder {
    fn to_sql(self) -> &'static str {
        match self {
            Self::NombreAsc  => "v.nombre COLLATE NOCASE ASC",
            Self::NombreDesc => "v.nombre COLLATE NOCASE DESC",
            Self::AnioAsc    => "v.anio_inicio ASC, v.nombre COLLATE NOCASE ASC",
            Self::AnioDesc   => "v.anio_inicio DESC, v.nombre COLLATE NOCASE ASC",
            Self::IssuesAsc  => "v.cantidad_numeros ASC, v.nombre COLLATE NOCASE ASC",
            Self::IssuesDesc => "v.cantidad_numeros DESC, v.nombre COLLATE NOCASE ASC",
        }
    }
}

pub struct VolumeRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> VolumeRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn get_all(&self) -> Result<Vec<Volume>> {
        let rows = sqlx::query_as::<_, Volume>(
            r#"SELECT
                id_volume,
                nombre,
                COALESCE(deck, '') as deck,
                COALESCE(descripcion, '') as descripcion,
                COALESCE(url, '') as url,
                COALESCE(image_url, '') as image_url,
                COALESCE(id_publisher, 0) as id_publisher,
                COALESCE(anio_inicio, 0) as anio_inicio,
                COALESCE(cantidad_numeros, 0) as cantidad_numeros,
                id_comicvine
               FROM volumens ORDER BY nombre COLLATE NOCASE"#
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn get_by_id(&self, id: i64) -> Result<Option<Volume>> {
        let row = sqlx::query_as::<_, Volume>(
            r#"SELECT
                id_volume,
                nombre,
                COALESCE(deck, '') as deck,
                COALESCE(descripcion, '') as descripcion,
                COALESCE(url, '') as url,
                COALESCE(image_url, '') as image_url,
                COALESCE(id_publisher, 0) as id_publisher,
                COALESCE(anio_inicio, 0) as anio_inicio,
                COALESCE(cantidad_numeros, 0) as cantidad_numeros,
                id_comicvine
               FROM volumens WHERE id_volume = ?"#
        )
        .bind(id)
        .fetch_optional(self.pool)
        .await?;
        Ok(row)
    }

    pub async fn get_by_publisher(&self, publisher_id: i64) -> Result<Vec<Volume>> {
        let rows = sqlx::query_as::<_, Volume>(
            r#"SELECT
                id_volume,
                nombre,
                COALESCE(deck, '') as deck,
                COALESCE(descripcion, '') as descripcion,
                COALESCE(url, '') as url,
                COALESCE(image_url, '') as image_url,
                COALESCE(id_publisher, 0) as id_publisher,
                COALESCE(anio_inicio, 0) as anio_inicio,
                COALESCE(cantidad_numeros, 0) as cantidad_numeros,
                id_comicvine
               FROM volumens WHERE id_publisher = ?"#
        )
        .bind(publisher_id)
        .fetch_all(self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn get_by_comicvine_id(&self, comicvine_id: i64) -> Result<Option<Volume>> {
        let row = sqlx::query_as::<_, Volume>(
            r#"SELECT
                id_volume,
                nombre,
                COALESCE(deck, '') as deck,
                COALESCE(descripcion, '') as descripcion,
                COALESCE(url, '') as url,
                COALESCE(image_url, '') as image_url,
                COALESCE(id_publisher, 0) as id_publisher,
                COALESCE(anio_inicio, 0) as anio_inicio,
                COALESCE(cantidad_numeros, 0) as cantidad_numeros,
                id_comicvine
               FROM volumens WHERE id_comicvine = ?"#
        )
        .bind(comicvine_id)
        .fetch_optional(self.pool)
        .await?;
        Ok(row)
    }

    pub async fn search(&self, query: &str) -> Result<Vec<Volume>> {
        let pattern = format!("%{}%", query);
        let rows = sqlx::query_as::<_, Volume>(
            r#"SELECT
                id_volume,
                nombre,
                COALESCE(deck, '') as deck,
                COALESCE(descripcion, '') as descripcion,
                COALESCE(url, '') as url,
                COALESCE(image_url, '') as image_url,
                COALESCE(id_publisher, 0) as id_publisher,
                COALESCE(anio_inicio, 0) as anio_inicio,
                COALESCE(cantidad_numeros, 0) as cantidad_numeros,
                id_comicvine
               FROM volumens
               WHERE nombre LIKE ? OR deck LIKE ?
               ORDER BY nombre COLLATE NOCASE"#
        )
        .bind(&pattern)
        .bind(&pattern)
        .fetch_all(self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn insert(&self, new: &NewVolume) -> Result<i64> {
        let result = sqlx::query(
            "INSERT INTO volumens
                (nombre, deck, descripcion, url, id_publisher, anio_inicio, cantidad_numeros, id_comicvine, image_url)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(&new.nombre)
        .bind(&new.deck)
        .bind(&new.descripcion)
        .bind(&new.url)
        .bind(new.id_publisher)
        .bind(new.anio_inicio)
        .bind(new.cantidad_numeros)
        .bind(new.id_comicvine)
        .bind(&new.image_url)
        .execute(self.pool)
        .await?;
        
        Ok(result.last_insert_rowid())
    }

    pub async fn update(&self, volume: &Volume) -> Result<()> {
        sqlx::query(
            "UPDATE volumens
             SET nombre = ?, deck = ?, descripcion = ?, url = ?, id_publisher = ?,
                 anio_inicio = ?, cantidad_numeros = ?, id_comicvine = ?, image_url = ?
             WHERE id_volume = ?"
        )
        .bind(&volume.nombre)
        .bind(&volume.deck)
        .bind(&volume.descripcion)
        .bind(&volume.url)
        .bind(volume.id_publisher)
        .bind(volume.anio_inicio)
        .bind(volume.cantidad_numeros)
        .bind(volume.id_comicvine)
        .bind(&volume.image_url)
        .bind(volume.id_volume)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete(&self, id: i64) -> Result<()> {
        sqlx::query("DELETE FROM volumens WHERE id_volume = ?")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn count(&self) -> Result<i64> {
        let row = sqlx::query(r#"SELECT COUNT(*) FROM volumens"#)
            .fetch_one(self.pool)
            .await?;
        Ok(row.get(0))
    }

    pub async fn count_completed(&self) -> Result<i64> {
        // Un volumen se considera completado si tenemos al menos tantos números
        // como indica cantidad_numeros (y cantidad_numeros > 0)
        let row = sqlx::query(
            r#"SELECT COUNT(*)
               FROM (
                   SELECT v.id_volume
                   FROM volumens v
                   JOIN comicbooks_info ci ON v.id_volume = ci.id_volume
                   GROUP BY v.id_volume
                   HAVING COUNT(ci.id_comicbook_info) >= v.cantidad_numeros AND v.cantidad_numeros > 0
               )"#
        )
        .fetch_one(self.pool)
        .await?;
        
        Ok(row.get(0))
    }

    /// Obtiene una página de VolumeView para la rejilla principal
    pub async fn get_page(
        &self,
        limit: i64,
        offset: i64,
        query: Option<&str>,
        sort: VolumeSortOrder,
        publisher_ids: &[i64],
    ) -> Result<Vec<crate::models::VolumeView>> {
        let parsed = query.map(parse_search_query).unwrap_or_default();
        let (search_sql, search_binds) = build_volume_search_fragment(&parsed);
        let order_sql = sort.to_sql();

        let pub_filter_sql = if publisher_ids.is_empty() {
            String::new()
        } else {
            let placeholders = publisher_ids.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
            format!("AND v.id_publisher IN ({})", placeholders)
        };

        let sql = format!(
            r#"SELECT
                v.id_volume,
                v.nombre,
                v.anio_inicio,
                v.cantidad_numeros,
                (SELECT COUNT(*)
                 FROM comicbooks_info ci
                 JOIN comicbooks cb ON ci.id_comicbook_info = cb.id_comicbook_info
                 WHERE ci.id_volume = v.id_volume) as cantidad_poseida,
                (SELECT cb.id_comicbook
                 FROM comicbooks cb
                 JOIN comicbooks_info ci ON cb.id_comicbook_info = ci.id_comicbook_info
                 WHERE ci.id_volume = v.id_volume
                 ORDER BY CAST(ci.numero AS REAL) ASC
                 LIMIT 1) as id_portada,
                (SELECT cb.path
                 FROM comicbooks cb
                 JOIN comicbooks_info ci ON cb.id_comicbook_info = ci.id_comicbook_info
                 WHERE ci.id_volume = v.id_volume
                 ORDER BY CAST(ci.numero AS REAL) ASC
                 LIMIT 1) as path_portada,
                v.image_url
               FROM volumens v
               WHERE 1=1
                 {search_sql}
                 {pub_filter_sql}
               ORDER BY {order_sql}
               LIMIT ? OFFSET ?"#
        );

        let mut q = sqlx::query(&sql);
        for bind in &search_binds {
            q = q.bind(bind);
        }
        for &id in publisher_ids {
            q = q.bind(id);
        }
        let rows = q
            .bind(limit)
            .bind(offset)
            .fetch_all(self.pool)
            .await?;

        Ok(rows.into_iter().map(|r| crate::models::VolumeView {
            id_volume: r.get(0),
            nombre: r.get(1),
            anio_inicio: r.get(2),
            cantidad_numeros: r.get(3),
            cantidad_poseida: r.get(4),
            id_comicbook_portada: r.get(5),
            path_portada: r.get(6),
            image_url: r.get(7),
        }).collect())
    }

    /// Editoriales que tienen al menos un volumen en la biblioteca
    pub async fn get_publishers_in_use(&self) -> Result<Vec<(i64, String)>> {
        let rows = sqlx::query(
            r#"SELECT p.id_publisher, p.nombre
               FROM publishers p
               WHERE p.id_publisher IN (
                   SELECT DISTINCT id_publisher FROM volumens WHERE id_publisher IS NOT NULL
               )
               ORDER BY p.nombre COLLATE NOCASE"#,
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| (r.get(0), r.get(1))).collect())
    }
}

// ---------------------------------------------------------------------------
// Helpers de búsqueda
// ---------------------------------------------------------------------------

/// Genera el fragmento WHERE y los binds para la búsqueda avanzada de volúmenes.
/// Columnas: `v.nombre` (título de la serie) y `v.deck` (descripción corta).
fn build_volume_search_fragment(
    parsed: &crate::models::ParsedQuery,
) -> (String, Vec<String>) {
    if parsed.is_empty() {
        return (String::new(), Vec::new());
    }

    const COLS: &str = "v.nombre LIKE ? OR v.deck LIKE ?";
    const NOT_COLS: &str = "v.nombre NOT LIKE ? AND v.deck NOT LIKE ?";

    let mut parts = Vec::new();
    let mut binds: Vec<String> = Vec::new();

    for word in &parsed.must_include {
        let pat = format!("%{}%", word);
        parts.push(format!("AND ({})", COLS));
        binds.extend([pat.clone(), pat]);
    }

    for word in &parsed.must_exclude {
        let pat = format!("%{}%", word);
        parts.push(format!("AND ({})", NOT_COLS));
        binds.extend([pat.clone(), pat]);
    }

    (parts.join("\n                 "), binds)
}
