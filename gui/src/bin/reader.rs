use adw::prelude::*;
use anyhow::Result;
use babelcomics::ui::reader::ReaderWindow;
use babelcomics_core::helpers::paths;
use gtk4::prelude::*;
use gtk4::{self as gtk, gio};
use libadwaita as adw;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("babelcomics=debug".parse()?),
        )
        .init();

    // Inicializar thumbnails con None (usará la ruta por defecto: ~/.local/share/babelcomics/thumbnails)
    // El standalone reader no lee de la BD para ser ligero e independiente.
    paths::initialize_thumbnails_base(None);

    // Limpiar cachés huérfanos de sesiones anteriores (crashes, apagones, etc.)
    babelcomics_core::helpers::extraction_registry::cleanup_stale();

    let app = adw::Application::builder()
        .application_id("com.github.babelcomics.standalone-reader")
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    app.connect_activate(move |app| {
        let args: Vec<String> = std::env::args().collect();

        // El primer argumento es el ejecutable, el segundo es el path al comic
        if args.len() > 1 {
            let path = &args[1];
            if std::path::Path::new(path).exists() {
                // Cargar CSS para que el lector se vea bien (estilos de sidebar, etc.)
                let provider = gtk::CssProvider::new();
                provider.load_from_string(include_str!("../ui/styles.css"));
                if let Some(display) = gtk::gdk::Display::default() {
                    gtk::style_context_add_provider_for_display(
                        &display,
                        &provider,
                        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
                    );
                }

                tracing::info!("Abriendo comic: {}", path);
                ReaderWindow::open(app, path, None);
            } else {
                eprintln!("Error: Archivo no encontrado: {}", path);
                std::process::exit(1);
            }
        } else {
            eprintln!("Uso: babelcomics-reader [PATH_AL_COMIC]");
            std::process::exit(1);
        }
    });

    // Soporte para abrir archivos desde el explorador de archivos (vía señal OPEN)
    app.connect_open(move |app, files, _| {
        for file in files {
            if let Some(path) = file.path() {
                let path_str = path.to_string_lossy().to_string();
                ReaderWindow::open(app, &path_str, None);
            }
        }
    });

    app.run();
    Ok(())
}
