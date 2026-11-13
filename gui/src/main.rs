use anyhow::Result;
use babelcomics::app;

#[tokio::main]
async fn main() -> Result<()> {
    let ctx = babelcomics_core::bootstrap::init().await?;

    // Arrancar la UI (bloqueante hasta que se cierre la ventana)
    app::run(ctx.pool);
    tracing::debug!("app::run() devolvió control a main; saliendo del proceso");

    // Salida limpia
    std::process::exit(0);
}
