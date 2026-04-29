use anyhow::Result;
use babelcomics::app;
use babelcomics_core::{db, helpers, repositories};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("babelcomics=debug".parse()?),
        )
        .init();

    let db_path = get_db_path();
    tracing::info!("Iniciando Babelcomics — BD: {}", db_path);

    let pool = db::create_pool(&db_path).await?;

    // Configurar Rayon para reservar 2 núcleos para la UI/Tareas inmediatas
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let rayon_threads = (cores as i32 - 2).max(1) as usize;
    tracing::info!(
        "Configurando Rayon con {} hilos (reservando 2 para UI)",
        rayon_threads
    );

    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(rayon_threads)
        .build_global();

    // Inicializar el directorio de thumbnails desde la configuración guardada
    let setup = repositories::SetupRepository::new(&pool).get().await?;
    helpers::paths::initialize_thumbnails_base(setup.carpeta_thumbnails);

    // Arrancar la UI (bloqueante hasta que se cierre la ventana)
    app::run(pool);

    // Salida limpia
    std::process::exit(0);
}

fn get_db_path() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    format!("{}/.local/share/babelcomics/babelcomics.db", home)
}
