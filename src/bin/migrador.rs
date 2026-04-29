use anyhow::Result;
use sqlx::{Row, sqlite::SqlitePoolOptions};

#[tokio::main]
async fn main() -> Result<()> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let base_path = format!("{}/.local/share/babelcomics", home);
    let old_db_path = format!("{}/babelcomics_python.db", base_path);
    let new_db_path = format!("{}/babelcomics.db", base_path);

    println!("🚀 Iniciando migración de datos...");
    println!("   Origen: {}", old_db_path);
    println!("   Destino: {}", new_db_path);

    if std::path::Path::new(&new_db_path).exists() {
        println!("⚠️  Borrando base de datos de destino existente...");
        std::fs::remove_file(&new_db_path)?;
    }

    let pool_new = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&format!("sqlite://{}?mode=rwc", new_db_path))
        .await?;

    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&pool_new)
        .await?;

    println!("📦 Inicializando esquema de Rust...");
    sqlx::migrate!("./migrations").run(&pool_new).await?;

    let pool_old = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&format!("sqlite://{}?mode=ro", old_db_path))
        .await?;

    // --- MIGRACIÓN: PUBLISHERS ---
    println!("📊 Migrando Publishers...");
    let old_publishers = sqlx::query(
        "SELECT id_publisher, nombre, descripcion, id_comicvine, url_logo FROM publishers",
    )
    .fetch_all(&pool_old)
    .await?;
    for row in old_publishers {
        sqlx::query("INSERT INTO publishers (id_publisher, nombre, descripcion, id_comicvine, image_url) VALUES (?, ?, ?, ?, ?)")
            .bind(row.get::<i64, _>("id_publisher"))
            .bind(row.get::<String, _>("nombre"))
            .bind(row.get::<Option<String>, _>("descripcion"))
            .bind(row.get::<Option<i64>, _>("id_comicvine"))
            .bind(row.get::<Option<String>, _>("url_logo"))
            .execute(&pool_new).await?;
    }

    // --- MIGRACIÓN: VOLUMENS ---
    println!("📚 Migrando Volúmenes...");
    let old_volumes = sqlx::query("SELECT id_volume, nombre, deck, descripcion, url, image_url, id_publisher, anio_inicio, cantidad_numeros, id_comicvine FROM volumens")
        .fetch_all(&pool_old).await?;
    for row in old_volumes {
        let pub_id: i64 = row.get("id_publisher");
        let pub_id_opt = if pub_id > 0 { Some(pub_id) } else { None };

        sqlx::query("INSERT INTO volumens (id_volume, nombre, deck, descripcion, url, image_url, id_publisher, anio_inicio, cantidad_numeros, id_comicvine) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(row.get::<i64, _>("id_volume"))
            .bind(row.get::<String, _>("nombre"))
            .bind(row.get::<String, _>("deck"))
            .bind(row.get::<String, _>("descripcion"))
            .bind(row.get::<String, _>("url"))
            .bind(row.get::<String, _>("image_url"))
            .bind(pub_id_opt)
            .bind(row.get::<i64, _>("anio_inicio"))
            .bind(row.get::<i64, _>("cantidad_numeros"))
            .bind(row.get::<Option<i64>, _>("id_comicvine"))
            .execute(&pool_new).await?;
    }

    // --- MIGRACIÓN: COMICBOOKS_INFO ---
    println!("📑 Migrando Metadatos (Comic Info)...");
    // NOTA: En Python es comicvine_id, en Rust es id_comicvine
    let old_info = sqlx::query("SELECT id_comicbook_info, titulo, id_volume, numero, resumen, calificacion, comicvine_id, url_api_detalle, fue_actualizado_api FROM comicbooks_info")
        .fetch_all(&pool_old).await?;
    for row in old_info {
        sqlx::query("INSERT INTO comicbooks_info (id_comicbook_info, titulo, id_volume, numero, resumen, calificacion, id_comicvine, url_api_detalle, fue_actualizado_api) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(row.get::<i64, _>("id_comicbook_info"))
            .bind(row.get::<String, _>("titulo"))
            .bind(row.get::<i64, _>("id_volume"))
            .bind(row.get::<String, _>("numero"))
            .bind(row.get::<Option<String>, _>("resumen"))
            .bind(row.get::<f64, _>("calificacion"))
            .bind(row.get::<i64, _>("comicvine_id"))
            .bind(row.get::<Option<String>, _>("url_api_detalle"))
            .bind(row.get::<bool, _>("fue_actualizado_api"))
            .execute(&pool_new).await?;
    }

    // --- MIGRACIÓN: COVERS ---
    println!("🖼️  Migrando Portadas Variantes...");
    let old_covers =
        sqlx::query("SELECT id_cover, id_comicbook_info, url_imagen FROM comicbooks_info_covers")
            .fetch_all(&pool_old)
            .await?;
    for row in old_covers {
        sqlx::query("INSERT INTO comicbooks_info_covers (id, id_comicbook_info, url_original) VALUES (?, ?, ?)")
            .bind(row.get::<i64, _>("id_cover"))
            .bind(row.get::<i64, _>("id_comicbook_info"))
            .bind(row.get::<String, _>("url_imagen"))
            .execute(&pool_new).await?;
    }

    // --- MIGRACIÓN: COMICBOOKS (Archivos) ---
    println!("💾 Migrando Archivos de Comics...");
    let old_cb = sqlx::query(
        "SELECT id_comicbook, path, id_comicbook_info, calidad, en_papelera FROM comicbooks",
    )
    .fetch_all(&pool_old)
    .await?;
    for row in old_cb {
        let info_id_raw: String = row.get(2);
        let info_id: Option<i64> = info_id_raw.parse::<i64>().ok().filter(|&id| id > 0);

        sqlx::query("INSERT INTO comicbooks (id_comicbook, path, id_comicbook_info, calidad, en_papelera) VALUES (?, ?, ?, ?, ?)")
            .bind(row.get::<i64, _>("id_comicbook"))
            .bind(row.get::<String, _>("path"))
            .bind(info_id)
            .bind(row.get::<i64, _>("calidad").to_string())
            .bind(row.get::<bool, _>("en_papelera"))
            .execute(&pool_new).await?;
    }

    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool_new)
        .await?;

    println!("✅ Migración completada con éxito.");
    Ok(())
}
