use anyhow::Result;
use sqlx::SqlitePool;

use crate::{db, helpers, repositories};

pub struct AppContext {
    pub pool: SqlitePool,
    pub thumbnails_base: Option<String>,
}

pub async fn init() -> Result<AppContext> {
    // Configurar Tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("babelcomics=debug".parse()?),
        )
        .init();

    let db_path = get_db_path();
    tracing::info!("Iniciando Babelcomics — BD: {}", db_path);

    let pool = db::create_pool(&db_path).await?;

    // Configurar Rayon con un límite conservador.
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let rayon_threads = 1;
    tracing::info!(
        "Configurando Rayon con {} hilos ({} cores detectados)",
        rayon_threads,
        cores
    );
    if rayon_threads < cores {
        tracing::info!(
            "Rayon limitado para mantener responsividad ({} cores reservados)",
            cores - rayon_threads
        );
    }
    tracing::debug!(
        "Rayon threadpool global inicializado con {} hilos",
        rayon_threads
    );

    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(rayon_threads)
        .build_global();

    // Inicializar el directorio de thumbnails desde la configuración guardada
    let setup = repositories::SetupRepository::new(&pool).get().await?;
    helpers::paths::initialize_thumbnails_base(setup.carpeta_thumbnails.clone());

    Ok(AppContext {
        pool,
        thumbnails_base: setup.carpeta_thumbnails,
    })
}

pub fn get_db_path() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    format!("{}/.local/share/babelcomics/babelcomics.db", home)
}
