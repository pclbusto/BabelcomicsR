use anyhow::Result;
use clap::{Parser, Subcommand};
use sqlx::{Row, sqlite::SqlitePoolOptions};

use babelcomics_core::bootstrap;
use babelcomics_core::helpers::scan_service;
use babelcomics_core::helpers::thumbnail::CardSize;

mod commands;

#[derive(Parser)]
#[command(author, version, about = "CLI para Babelcomics", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Lista entidades de la biblioteca (comics, volúmenes, editoriales)
    List {
        /// Tipo de entidad: comics, volumes, publishers
        entity: String,
        /// Número de página (empieza en 1)
        #[arg(short = 'n', long, default_value_t = 1)]
        page: u32,
        /// Cantidad de resultados por página
        #[arg(short = 'p', long, default_value_t = 20)]
        per_page: u32,
        /// Filtrar por editorial (búsqueda parcial, case-insensitive)
        #[arg(short = 'P', long)]
        publisher: Option<String>,
        /// Filtrar por volumen (búsqueda parcial, case-insensitive)
        #[arg(short = 'V', long)]
        volume: Option<String>,
        /// Filtrar por título (búsqueda parcial, case-insensitive)
        #[arg(short = 't', long)]
        title: Option<String>,
        /// Filtrar por texto en el path del archivo (búsqueda parcial, case-insensitive)
        #[arg(short = 'f', long)]
        path: Option<String>,
        /// Mostrar solo cómics en la papelera
        #[arg(short = 'T', long)]
        trash: bool,
        /// Mostrar solo cómics sin calificación
        #[arg(short = 'u', long)]
        unread: bool,
        /// Mostrar solo cómics no procesados
        #[arg(short = 'U', long)]
        unprocessed: bool,
        /// Mostrar solo cómics no catalogados (sin metadatos ComicVine)
        #[arg(short = 'C', long)]
        uncataloged: bool,
    },
    /// Escanea un directorio en busca de cómics
    Scan {
        /// Directorio a escanear
        dir: String,
    },
    /// Tareas de miniaturas
    Thumbnail {
        #[command(subcommand)]
        cmd: ThumbnailCommands,
    },
    /// Operaciones de base de datos
    Db {
        #[command(subcommand)]
        cmd: DbCommands,
    },
    /// Búsqueda en ComicVine
    Comicvine {
        #[command(subcommand)]
        cmd: CvCommands,
    },
    /// Modo interactivo REPL
    Repl,
}

#[derive(Subcommand)]
enum ThumbnailCommands {
    /// Genera miniaturas faltantes para todos los comics
    Generate,
}

#[derive(Subcommand)]
enum DbCommands {
    /// Muestra información estadística de la base de datos
    Info,
    /// Migra la base de datos de la versión antigua (Python) a Rust
    Migrate,
}

#[derive(Subcommand)]
enum CvCommands {
    /// Busca volúmenes
    Search { query: String },
}

pub struct ListFilters {
    pub publisher: Option<String>,
    pub volume: Option<String>,
    pub title: Option<String>,
    pub path: Option<String>,
    pub trash: bool,
    pub unread: bool,
    pub unprocessed: bool,
    pub uncataloged: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Migrar no necesita el pool normal
    if let Commands::Db {
        cmd: DbCommands::Migrate,
    } = &cli.command
    {
        return run_migration().await;
    }

    let ctx = bootstrap::init().await?;

    match &cli.command {
        Commands::List {
            entity,
            page,
            per_page,
            publisher,
            volume,
            title,
            path,
            trash,
            unread,
            unprocessed,
            uncataloged,
        } => {
            let filters = ListFilters {
                publisher: publisher.clone(),
                volume: volume.clone(),
                title: title.clone(),
                path: path.clone(),
                trash: *trash,
                unread: *unread,
                unprocessed: *unprocessed,
                uncataloged: *uncataloged,
            };
            match entity.as_str() {
                "comics" | "c" => {
                    commands::list::list_comics(&ctx.pool, *page, *per_page, &filters, &[]).await?;
                }
                "volumes" | "v" => {
                    commands::list::list_volumes(&ctx.pool, *page, *per_page, publisher, &[])
                        .await?;
                }
                "publishers" | "p" => {
                    commands::list::list_publishers(&ctx.pool, *page, *per_page, &[]).await?;
                }
                _ => {
                    eprintln!(
                        "Entidad desconocida: {}. Usá 'comics', 'volumes' o 'publishers'.",
                        entity
                    );
                }
            }
        }
        Commands::Scan { dir } => {
            println!("Escaneando directorio: {}", dir);
            let dirs = vec![dir.clone()];
            let result = scan_service::run_scan(&ctx.pool, &dirs, CardSize::Medium).await?;
            println!(
                "Escaneo finalizado. Encontrados: {}, Nuevos insertados: {}, Portadas generadas: {}, Errores: {}",
                result.total_found,
                result.new_inserted,
                result.covers_generated,
                result.errors.len()
            );
        }
        Commands::Thumbnail { cmd } => match cmd {
            ThumbnailCommands::Generate => {
                println!("Generando thumbnails faltantes...");
                let result =
                    scan_service::generate_missing_thumbnails(&ctx.pool, CardSize::Medium).await?;
                println!(
                    "Generación finalizada. Generados: {}, Errores: {}",
                    result.covers_generated,
                    result.errors.len()
                );
            }
        },
        Commands::Db { cmd } => match cmd {
            DbCommands::Info => {
                println!("Obteniendo info de la BD...");
                let total_comics: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM comicbooks")
                    .fetch_one(&ctx.pool)
                    .await?;
                let total_volumes: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM volumens")
                    .fetch_one(&ctx.pool)
                    .await?;
                let total_publishers: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM publishers")
                    .fetch_one(&ctx.pool)
                    .await?;
                let total_info: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM comicbooks_info")
                    .fetch_one(&ctx.pool)
                    .await?;
                println!("Comics:      {}", total_comics);
                println!("Volúmenes:   {}", total_volumes);
                println!("Editoriales: {}", total_publishers);
                println!("Metadatos:   {}", total_info);
            }
            DbCommands::Migrate => {
                // Manejado arriba
            }
        },
        Commands::Comicvine { cmd } => match cmd {
            CvCommands::Search { query } => {
                println!(
                    "Para buscar '{}' en ComicVine, la CLI necesita acceder a la configuración de la API Key, lo cual está pendiente de implementar en este comando.",
                    query
                );
            }
        },
        Commands::Repl => {
            commands::repl::run(&ctx.pool).await?;
        }
    }

    Ok(())
}

async fn run_migration() -> Result<()> {
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
    sqlx::migrate!("../core/migrations").run(&pool_new).await?;

    let pool_old = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&format!("sqlite://{}?mode=ro", old_db_path))
        .await?;

    // MIGRACIÓN: PUBLISHERS
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

    // MIGRACIÓN: VOLUMENS
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

    // MIGRACIÓN: COMICBOOKS_INFO
    println!("📑 Migrando Metadatos (Comic Info)...");
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

    // MIGRACIÓN: COVERS
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

    // MIGRACIÓN: COMICBOOKS
    println!("💾 Migrando Archivos de Comics...");
    let old_cb = sqlx::query(
        "SELECT id_comicbook, path, id_comicbook_info, calidad, en_papelera FROM comicbooks",
    )
    .fetch_all(&pool_old)
    .await?;
    for row in old_cb {
        let info_id_raw: String = row.get::<String, _>(2);
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
