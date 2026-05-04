use std::cell::OnceCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{self as gtk, gio, glib};
use libadwaita as adw;
use sqlx::SqlitePool;

use crate::ui::{self, reader, window};
use babelcomics_core::helpers::{scan_service, thumbnail::CardSize};
use babelcomics_core::repositories::SetupRepository;

const APP_ID: &str = "com.github.babelcomics";

pub fn run(pool: SqlitePool) {
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    let pool_cell: Rc<OnceCell<SqlitePool>> = Rc::new(OnceCell::new());
    let _ = pool_cell.set(pool);

    let pool_cell2 = pool_cell.clone();
    let pool_cell_open = pool_cell.clone();

    // Señal OPEN: se dispara cuando el sistema pide abrir archivos (ej: doble clic en Nautilus)
    {
        app.connect_open(move |app, files, _| {
            for file in files {
                if let Some(path) = file.path() {
                    let path_str = path.to_string_lossy().to_string();
                    let pool = pool_cell_open.get().cloned();
                    reader::ReaderWindow::open(app, &path_str, pool);
                }
            }
        });
    }

    // Cache compartido: connect_activate lo pre-construye en idle,
    // setup_actions lo usa en cada apertura.
    let prefs_cache: Rc<OnceCell<adw::PreferencesDialog>> = Rc::new(OnceCell::new());
    let prefs_cache_activate = prefs_cache.clone();

    app.connect_activate(move |app| {
        // Limpiar cachés huérfanos de sesiones anteriores (crashes, apagones, etc.)
        babelcomics_core::helpers::extraction_registry::cleanup_stale();

        // Cargar CSS
        let provider = gtk::CssProvider::new();
        provider.load_from_string(include_str!("ui/styles.css"));
        gtk::style_context_add_provider_for_display(
            &gtk::gdk::Display::default().expect("No se pudo conectar a un display."),
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        // Soporte para argumentos manuales (backup de connect_open)
        let args: Vec<String> = std::env::args().collect();
        if args.len() > 1 {
            let path = &args[1];
            // Verificar si el archivo existe antes de intentar abrirlo
            if std::path::Path::new(path).exists() {
                tracing::info!("Abriendo comic desde argumento: {}", path);
                let pool = pool_cell2.get().cloned();
                reader::ReaderWindow::open(app, path, pool);
                return;
            }
        }

        if app.windows().is_empty() {
            let pool = pool_cell2.get().unwrap().clone();
            let win = window::build(app, pool.clone());
            win.present();

            // Pre-construir el diálogo de preferencias en el primer ciclo idle
            // para que esté listo cuando el usuario lo abra (sin bloquear el arranque).
            {
                let cache = prefs_cache_activate.clone();
                let pool_prefs = pool.clone();
                glib::idle_add_local_once(move || {
                    let _ = cache.set(ui::build_preferences_dialog(pool_prefs));
                });
            }

            // Tareas de mantenimiento al arranque (en segundo plano)
            let pool_boot = pool.clone();
            ui::run_in_background(
                tokio::runtime::Handle::current(),
                async move {
                    let setup = SetupRepository::new(&pool_boot)
                        .get()
                        .await
                        .unwrap_or_default();

                    // 1. Reparar thumbnails que quedaron a medias
                    let card_size = CardSize::from_db(setup.thumbnail_size);
                    let thumb_res =
                        scan_service::generate_missing_thumbnails(&pool_boot, card_size).await;

                    // 2. Generar embeddings CLIP (solo si el usuario lo tiene activado)
                    let clip_res = if setup.clip_al_arranque {
                        Some(scan_service::generate_missing_clip_embeddings(&pool_boot).await)
                    } else {
                        None
                    };

                    (thumb_res, clip_res)
                },
                |(thumb_res, clip_res)| {
                    if let Ok(r) = thumb_res {
                        if r.covers_generated > 0 {
                            tracing::info!(
                                "Thumbnails faltantes regenerados al arranque: {}",
                                r.covers_generated
                            );
                        }
                    }
                    if let Some(Ok((generated, _))) = clip_res {
                        if generated > 0 {
                            tracing::info!("Embeddings CLIP generados al arranque: {}", generated);
                        }
                    }
                },
            );
        } else {
            app.windows()[0].present();
        }
    });

    setup_actions(&app, pool_cell, prefs_cache);

    app.run();

    // Al salir de app.run() (ventana cerrada), activamos el flag de parada
    scan_service::STOP_THREADS.store(true, std::sync::atomic::Ordering::Relaxed);
}

fn setup_actions(
    app: &adw::Application,
    pool_cell: Rc<OnceCell<SqlitePool>>,
    prefs_cache: Rc<OnceCell<adw::PreferencesDialog>>,
) {
    // app.scan
    let scan_action = gio::SimpleAction::new("scan", None);
    scan_action.connect_activate(|_, _| {
        tracing::info!("Escaneo iniciado desde acción global");
    });
    app.add_action(&scan_action);

    // app.preferences — el diálogo se pre-construye en idle al arrancar y se reutiliza
    let prefs_action = gio::SimpleAction::new("preferences", None);
    {
        let pool_cell = pool_cell.clone();
        prefs_action.connect_activate(move |_, _| {
            let Some(app) = gtk4::gio::Application::default() else {
                return;
            };
            let Some(app) = app.downcast_ref::<adw::Application>() else {
                return;
            };
            let win = app.active_window();
            let pool = pool_cell.get().unwrap().clone();
            let dialog = prefs_cache.get_or_init(|| ui::build_preferences_dialog(pool));
            dialog.present(win.as_ref());
        });
    }
    app.add_action(&prefs_action);

    // app.about
    let about_action = gio::SimpleAction::new("about", None);
    about_action.connect_activate(|_, _| {
        let dialog = adw::AboutDialog::builder()
            .application_name("Babelcomics")
            .version("0.1.0")
            .developer_name("Pedro")
            .license_type(gtk4::License::Gpl30)
            .website("https://github.com/babelcomics")
            .issue_url("https://github.com/babelcomics/issues")
            .build();

        if let Some(app) = gtk4::gio::Application::default() {
            if let Some(win) = app
                .downcast_ref::<adw::Application>()
                .and_then(|a| a.active_window())
            {
                dialog.present(Some(&win));
            }
        }
    });
    app.add_action(&about_action);

    app.set_accels_for_action("app.scan", &["<Control>r"]);
    app.set_accels_for_action("app.preferences", &["<Control>comma"]);
    app.set_accels_for_action("app.about", &[]);
}
