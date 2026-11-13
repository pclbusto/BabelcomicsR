use std::cell::OnceCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{self as gtk, gio, glib};
use libadwaita as adw;
use sqlx::SqlitePool;

use crate::ui::{self, reader, window};
use babelcomics_core::helpers::scan_service;

const APP_ID: &str = "com.github.babelcomics";
static APP_RUN_RETURNED: AtomicBool = AtomicBool::new(false);
static SHUTDOWN_WATCHDOG_STARTED: AtomicBool = AtomicBool::new(false);

pub fn run(pool: SqlitePool) {
    APP_RUN_RETURNED.store(false, Ordering::Relaxed);
    SHUTDOWN_WATCHDOG_STARTED.store(false, Ordering::Relaxed);

    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    let force_activate_without_files = std::env::args_os().len() == 1;
    app.connect_startup(move |app| {
        tracing::debug!(is_remote = app.is_remote(), "Señal startup de GTK recibida");

        if force_activate_without_files {
            let app = app.clone();
            glib::idle_add_local_once(move || {
                if app.windows().is_empty() {
                    tracing::debug!(
                        is_remote = app.is_remote(),
                        "Sin ventana tras startup; forzando activate()"
                    );
                    app.activate();
                }
            });
        }
    });

    app.connect_window_removed(|app, window| {
        let windows = app.windows().len();
        tracing::debug!(
            windows,
            active_background_tasks = ui::active_background_tasks(),
            title = ?window.title(),
            "Ventana removida de la aplicación"
        );
        if windows == 0 {
            ui::log_pending_background_callbacks("última ventana removida");
            ui::abort_pending_background_callbacks("última ventana removida");
        }
    });

    app.connect_shutdown(|_| {
        tracing::debug!(
            active_background_tasks = ui::active_background_tasks(),
            "Señal shutdown de GTK recibida"
        );
        ui::log_pending_background_callbacks("shutdown GTK");
        ui::abort_pending_background_callbacks("shutdown GTK");
        scan_service::STOP_THREADS.store(true, Ordering::Relaxed);

        if !SHUTDOWN_WATCHDOG_STARTED.swap(true, Ordering::Relaxed) {
            std::thread::spawn(|| {
                std::thread::sleep(Duration::from_millis(750));
                if !APP_RUN_RETURNED.load(Ordering::Relaxed) {
                    eprintln!(
                        "Babelcomics: GTK emitió shutdown pero app.run() no retornó; forzando salida inmediata."
                    );
                    force_process_exit(0);
                }
            });
        }
    });

    let pool_cell: Rc<OnceCell<SqlitePool>> = Rc::new(OnceCell::new());
    let _ = pool_cell.set(pool);

    let pool_cell2 = pool_cell.clone();
    let pool_cell_open = pool_cell.clone();

    // Señal OPEN: se dispara cuando el sistema pide abrir archivos (ej: doble clic en Nautilus)
    {
        app.connect_open(move |app, files, _| {
            tracing::debug!(
                is_remote = app.is_remote(),
                file_count = files.len(),
                "Señal open de GTK recibida"
            );
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
        tracing::debug!(
            is_remote = app.is_remote(),
            windows = app.windows().len(),
            "Señal activate de GTK recibida"
        );
        ui::install_main_loop_stall_monitor();

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

            tracing::debug!(
                "Mantenimiento pesado de thumbnails/CLIP omitido al arranque; se ejecuta bajo demanda"
            );
        } else {
            app.windows()[0].present();
        }
    });

    setup_actions(&app, pool_cell, prefs_cache);

    tracing::debug!("Entrando a app.run()");
    app.run();
    APP_RUN_RETURNED.store(true, Ordering::Relaxed);
    tracing::debug!(
        active_background_tasks = ui::active_background_tasks(),
        "app.run() terminó"
    );

    // Al salir de app.run() (ventana cerrada), activamos el flag de parada
    scan_service::STOP_THREADS.store(true, Ordering::Relaxed);
    tracing::debug!("STOP_THREADS activado");
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

#[cfg(unix)]
fn force_process_exit(code: i32) -> ! {
    unsafe extern "C" {
        fn _exit(status: i32) -> !;
    }

    unsafe { _exit(code) }
}

#[cfg(not(unix))]
fn force_process_exit(code: i32) -> ! {
    std::process::exit(code);
}
