use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{self as gtk, gio, glib};
use libadwaita as adw;
use sqlx::SqlitePool;
use std::sync::mpsc::Sender;

use super::pages;

#[derive(Clone, Debug)]
pub(crate) struct ClipProgressStats {
    pub total: i64,
    pub con_archivo: i64,
    pub indexadas: i64,
    pub pendientes: i64,
}

#[derive(Clone, Debug)]
pub(crate) enum ClipGenerationEvent {
    Progress(crate::helpers::scan_service::ClipGenerationProgress),
    Finished {
        result: Result<(u32, Vec<String>), String>,
        stats: ClipProgressStats,
    },
}

/// Construye y devuelve la ventana principal de la aplicación.
pub fn build(app: &adw::Application, pool: SqlitePool) -> adw::ApplicationWindow {
    let win = adw::ApplicationWindow::builder()
        .application(app)
        .title("Babelcomics")
        .default_width(1200)
        .default_height(800)
        .build();

    // TabView — contenedor principal de tabs
    let tab_view = adw::TabView::new();

    // TabBar — la tira de pestañas debajo del header
    let tab_bar = adw::TabBar::new();
    tab_bar.set_view(Some(&tab_view));
    tab_bar.set_autohide(false);

    // HeaderBar
    let header = adw::HeaderBar::new();

    // Botón que abre el TabOverview (miniaturas) — debe ser descendiente del TabOverview
    let tab_button = adw::TabButton::new();
    tab_button.set_view(Some(&tab_view));
    tab_button.set_action_name(Some("overview.open"));
    header.pack_start(&tab_button);

    // Botón de nueva tab → popover para elegir tipo
    let new_tab_btn = gtk::MenuButton::builder()
        .icon_name("tab-new-symbolic")
        .tooltip_text("Nueva pestaña")
        .popover(&build_tab_type_popover(&tab_view, &pool))
        .build();
    header.pack_start(&new_tab_btn);

    // Menú de hamburguesa
    let menu_model = build_menu();
    let menu_btn = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text("Menú principal")
        .menu_model(&menu_model)
        .build();
    header.pack_end(&menu_btn);

    // Botón de preferencias
    let prefs_btn = gtk::Button::builder()
        .icon_name("preferences-system-symbolic")
        .tooltip_text("Preferencias")
        .action_name("app.preferences")
        .build();
    header.pack_end(&prefs_btn);

    // Botón de estadísticas (información)
    let stats_btn = gtk::MenuButton::builder()
        .icon_name("help-about-symbolic")
        .tooltip_text("Estadísticas")
        .popover(&pages::statistics::build_popover(pool.clone()))
        .build();
    header.pack_end(&stats_btn);

    // ToolbarView: header + tabbar + tab_view (sin overview aquí)
    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.add_top_bar(&tab_bar);
    toolbar_view.set_content(Some(&tab_view));

    // TabOverview envuelve el toolbar_view completo → el header queda dentro
    // y la acción "overview.open" del TabButton puede resolverse correctamente
    let tab_overview = adw::TabOverview::new();
    tab_overview.set_view(Some(&tab_view));
    tab_overview.set_enable_new_tab(true);
    tab_overview.set_child(Some(&toolbar_view));

    // Conectar "nueva tab" desde el overview → crea una pestaña selector
    let pool_for_overview = pool.clone();
    tab_overview.connect_create_tab(glib::clone!(
        #[weak]
        tab_view,
        #[upgrade_or_panic]
        move |_| { add_tab(&tab_view, TabKind::Selector(pool_for_overview.clone())) }
    ));

    // ToastOverlay: envuelve el contenido para poder mostrar notificaciones
    let toast_overlay = adw::ToastOverlay::new();
    toast_overlay.set_child(Some(&tab_overview));
    win.set_content(Some(&toast_overlay));

    // Abrir tabs por defecto
    add_tab(&tab_view, TabKind::Comics(pool.clone()));
    add_tab(&tab_view, TabKind::Volumes(pool.clone()));
    add_tab(&tab_view, TabKind::Publishers);
    add_tab(&tab_view, TabKind::Downloads(pool.clone()));

    // Atajos de teclado
    setup_shortcuts(&win, &tab_view, &tab_overview);

    // Acción de ventana: generar embeddings CLIP manualmente
    setup_generate_clip_action(&win, &toast_overlay, pool.clone());

    // Acción de ventana: descargar portadas faltantes + generar embeddings
    setup_download_covers_action(&win, &toast_overlay, pool);

    win.maximize();

    win
}

/// Registra la acción `win.download-covers` que busca portadas ya descargadas en disco,
/// las enlaza en la BD si falta `ruta_local`, y luego genera los embeddings CLIP.
/// No descarga ningún archivo nuevo.
fn setup_download_covers_action(
    win: &adw::ApplicationWindow,
    toast_overlay: &adw::ToastOverlay,
    pool: SqlitePool,
) {
    let action = gio::SimpleAction::new("download-covers", None);

    let overlay_weak = toast_overlay.downgrade();
    let action_ref = action.downgrade();

    action.connect_activate(move |act, _| {
        act.set_enabled(false);

        if let Some(overlay) = overlay_weak.upgrade() {
            overlay.add_toast(
                adw::Toast::builder()
                    .title("Indexando portadas existentes…")
                    .timeout(0)
                    .build(),
            );
        }

        let pool_task = pool.clone();
        let overlay_done = overlay_weak.clone();
        let action_done = action_ref.clone();

        crate::ui::run_in_background(
            tokio::runtime::Handle::current(),
            async move {
                // 1. Enlazar en BD las portadas que ya están en disco
                let link_result =
                    crate::helpers::scan_service::relink_covers_from_disk(&pool_task).await;
                // 2. Generar embeddings CLIP para todas las que tienen ruta_local
                let clip_result =
                    crate::helpers::scan_service::generate_missing_clip_embeddings(&pool_task)
                        .await;
                (link_result, clip_result)
            },
            move |(link_result, clip_result)| {
                if let Some(a) = action_done.upgrade() {
                    a.set_enabled(true);
                }

                if let Some(overlay) = overlay_done.upgrade() {
                    let msg = match (link_result, clip_result) {
                        (Ok((0, _)), Ok((0, _))) => {
                            "No se encontraron portadas nuevas en disco".to_string()
                        }
                        (Ok((lk, _)), Ok((cl, errs))) => {
                            let mut parts = Vec::new();
                            if lk > 0 {
                                parts.push(format!("{} portadas enlazadas desde disco", lk));
                            }
                            if cl > 0 {
                                parts.push(format!("{} embeddings CLIP generados", cl));
                            }
                            if !errs.is_empty() {
                                parts.push(format!("{} errores (ver log)", errs.len()));
                            }
                            if parts.is_empty() {
                                "Todo ya estaba indexado".to_string()
                            } else {
                                parts.join(", ")
                            }
                        }
                        (Err(e), _) => format!("Error escaneando disco: {e}"),
                        (_, Err(e)) => format!("Error generando CLIP: {e}"),
                    };
                    overlay.add_toast(adw::Toast::builder().title(&msg).timeout(8).build());
                }
            },
        );
    });

    win.add_action(&action);
}

/// Registra la acción `win.generate-clip` que lanza la indexación CLIP en segundo plano.
/// Antes de comenzar muestra un diálogo que pregunta si indexar solo las portadas faltantes
/// o reindexar todo.
fn setup_generate_clip_action(
    win: &adw::ApplicationWindow,
    toast_overlay: &adw::ToastOverlay,
    pool: SqlitePool,
) {
    let action = gio::SimpleAction::new("generate-clip", None);

    let overlay_weak = toast_overlay.downgrade();
    let win_weak = win.downgrade();

    action.connect_activate(move |_act, _| {
        let dialog = adw::AlertDialog::builder()
            .heading("Generar embeddings CLIP")
            .body("¿Qué portadas quieres indexar?")
            .build();
        dialog.add_response("cancel", "Cancelar");
        dialog.add_response("missing", "Solo faltantes");
        dialog.add_response("all", "Reindexar todo");
        dialog.set_default_response(Some("missing"));
        dialog.set_close_response("cancel");
        dialog.set_response_appearance("all", adw::ResponseAppearance::Destructive);

        let overlay_clone = overlay_weak.clone();
        let pool_clone = pool.clone();

        dialog.connect_response(None, move |_d, response| {
            if response == "cancel" {
                return;
            }
            let solo_faltantes = response != "all";
            run_clip_generation(
                None,
                solo_faltantes,
                pool_clone.clone(),
                Some(overlay_clone.clone()),
                None,
            );
        });

        dialog.present(win_weak.upgrade().as_ref());
    });

    win.add_action(&action);
}

/// Lanza la generación de embeddings CLIP en background y muestra toasts de progreso/resultado.
///
/// - `volume_id`: si es `Some`, limita la indexación a ese volumen.
/// - `solo_faltantes`: si es `false`, reindexará aunque ya exista embedding.
/// - `overlay_weak`: si es `None`, no se muestran toasts.
pub(crate) fn run_clip_generation(
    volume_id: Option<i64>,
    solo_faltantes: bool,
    pool: SqlitePool,
    overlay_weak: Option<glib::WeakRef<adw::ToastOverlay>>,
    event_tx: Option<Sender<ClipGenerationEvent>>,
) {
    if let Some(ref ow) = overlay_weak {
        if let Some(overlay) = ow.upgrade() {
            let msg = if solo_faltantes {
                "Indexando portadas faltantes…"
            } else {
                "Reindexando todas las portadas…"
            };
            overlay.add_toast(adw::Toast::builder().title(msg).timeout(3).build());
        }
    }

    let event_tx_for_task = event_tx.clone();
    let event_tx_for_done = event_tx.clone();

    crate::ui::run_in_background(
        tokio::runtime::Handle::current(),
        async move {
            let info_repo = crate::repositories::ComicbookInfoRepository::new(&pool);
            let stats = info_repo
                .count_clip_index_stats(volume_id)
                .await
                .unwrap_or((0, 0, 0, 0));
            tracing::info!(
                "CLIP índice — total covers: {}, con archivo: {}, indexadas: {}, pendientes: {}",
                stats.0,
                stats.1,
                stats.2,
                stats.3
            );
            let progress_tx = event_tx_for_task.as_ref().map(|tx| {
                let ui_tx = tx.clone();
                let (progress_tx, progress_rx) = std::sync::mpsc::channel::<
                    crate::helpers::scan_service::ClipGenerationProgress,
                >();
                std::thread::spawn(move || {
                    while let Ok(progress) = progress_rx.recv() {
                        let _ = ui_tx.send(ClipGenerationEvent::Progress(progress));
                    }
                });
                progress_tx
            });
            let result = crate::helpers::scan_service::generate_clip_embeddings(
                &pool,
                volume_id,
                solo_faltantes,
                progress_tx,
            )
            .await;
            (result, stats)
        },
        move |(result, stats)| {
            let stats = ClipProgressStats {
                total: stats.0,
                con_archivo: stats.1,
                indexadas: stats.2,
                pendientes: stats.3,
            };
            if let Some(tx) = &event_tx_for_done {
                let _ = tx.send(ClipGenerationEvent::Finished {
                    result: result
                        .as_ref()
                        .map(|(generated, errs)| (*generated, errs.clone()))
                        .map_err(|e| e.to_string()),
                    stats: stats.clone(),
                });
            }

            let Some(ref ow) = overlay_weak else { return };
            let Some(overlay) = ow.upgrade() else { return };
            let total = stats.total;
            let con_archivo = stats.con_archivo;
            let indexadas = stats.indexadas;
            let pendientes = stats.pendientes;
            let msg = match result {
                Ok((0, _)) if con_archivo == 0 => {
                    format!("Sin portadas descargadas ({total} issues en BD).")
                }
                Ok((0, _)) => {
                    format!("{indexadas} portadas ya indexadas de {con_archivo} con archivo local")
                }
                Ok((n, errs)) if errs.is_empty() => format!(
                    "CLIP: {n} portadas indexadas ({total} total, {} pendientes)",
                    pendientes.saturating_sub(n as i64)
                ),
                Ok((n, errs)) => format!("CLIP: {n} indexadas, {} errores (ver log)", errs.len()),
                Err(e) => format!("CLIP error: {e}"),
            };
            overlay.add_toast(adw::Toast::builder().title(&msg).timeout(6).build());
        },
    );
}

// ---------------------------------------------------------------------------
// Tipos de tab
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub enum TabKind {
    Comics(SqlitePool),
    Volumes(SqlitePool),
    Publishers,
    Downloads(SqlitePool),
    ComicVineSearch(SqlitePool),
    /// Página temporal que permite elegir el tipo de tab
    Selector(SqlitePool),
    /// Detalle de un cómic específico
    ComicDetail(i64, SqlitePool),
    /// Detalle de un volumen (serie) específico
    VolumeDetail(i64, SqlitePool),
    /// Detalle de un issue (número) específico
    IssueDetail(i64, SqlitePool),
    /// Catalogación inteligente por similitud visual CLIP
    CatalogacionInteligente(Vec<i64>, SqlitePool),
}

/// Crea una nueva pestaña del tipo dado y la añade al TabView.
/// Devuelve la AdwTabPage creada (requerido por connect_create_tab).
pub fn add_tab(tab_view: &adw::TabView, kind: TabKind) -> adw::TabPage {
    // ComicDetail necesita actualizar el título de forma asíncrona tras crear la página.
    if let TabKind::ComicDetail(comic_id, pool) = kind {
        let content = pages::comic_detail::build(comic_id, pool.clone());
        let page = tab_view.append(&content);
        page.set_title("Detalle de cómic");
        page.set_icon(Some(&gio::ThemedIcon::new("image-x-generic-symbolic")));
        page.set_needs_attention(false);
        pages::comic_detail::setup_tab_title(comic_id, pool, page.downgrade());
        return page;
    }

    if let TabKind::VolumeDetail(volume_id, pool) = kind {
        let content = pages::volume_detail::build(volume_id, pool);
        let page = tab_view.append(&content);
        page.set_title("Detalle de serie");
        page.set_icon(Some(&gio::ThemedIcon::new("open-book-symbolic")));
        page.set_needs_attention(false);
        // Aquí podríamos añadir un setup_tab_title similar al de comics si quisiéramos
        return page;
    }

    if let TabKind::IssueDetail(issue_info_id, pool) = kind {
        let content = pages::issue_detail::build(issue_info_id, pool.clone());
        let page = tab_view.append(&content);
        page.set_title("Detalle de issue");
        page.set_icon(Some(&gio::ThemedIcon::new("image-x-generic-symbolic")));
        page.set_needs_attention(false);
        pages::issue_detail::setup_tab_title(issue_info_id, pool, page.downgrade());
        return page;
    }

    if let TabKind::CatalogacionInteligente(ids, pool) = kind {
        let content = pages::catalogacion_inteligente::build(ids, pool);
        let page = tab_view.append(&content);
        page.set_title("Catalogación Inteligente");
        page.set_icon(Some(&gio::ThemedIcon::new("find-location-symbolic")));
        page.set_needs_attention(false);
        return page;
    }

    let (title, icon_name, content): (String, &str, gtk::Widget) = match kind {
        TabKind::Comics(pool) => (
            "Comics".into(),
            "image-x-generic-symbolic",
            pages::comics::build(pool, tab_view.clone()),
        ),
        TabKind::Volumes(pool) => (
            "Series".into(),
            "accessories-dictionary-symbolic",
            pages::volumes::build(pool, tab_view.clone()),
        ),

        TabKind::Publishers => (
            "Editoriales".into(),
            "building-symbolic",
            pages::publishers::build(),
        ),
        TabKind::Downloads(pool) => (
            "Descargas".into(),
            "folder-download-symbolic",
            pages::downloads::build(pool),
        ),
        TabKind::ComicVineSearch(pool) => (
            "Buscar en ComicVine".into(),
            "web-browser-symbolic",
            pages::comicvine_search::build(pool),
        ),
        TabKind::Selector(pool) => (
            "Nueva pestaña".into(),
            "tab-new-symbolic",
            build_selector_page(tab_view, pool),
        ),
        TabKind::ComicDetail(..) => unreachable!(),
        TabKind::VolumeDetail(..) => unreachable!(),
        TabKind::IssueDetail(..) => unreachable!(),
        TabKind::CatalogacionInteligente(..) => unreachable!(),
    };

    let page = tab_view.append(&content);
    page.set_title(&title);
    page.set_icon(Some(&gio::ThemedIcon::new(icon_name)));
    page.set_needs_attention(false);
    page
}

// ---------------------------------------------------------------------------
// Popover de selección de tipo de tab (botón nueva tab en el header)
// ---------------------------------------------------------------------------

fn build_tab_type_popover(tab_view: &adw::TabView, pool: &SqlitePool) -> gtk::Popover {
    let popover = gtk::Popover::new();

    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(6)
        .margin_end(6)
        .build();

    let kinds: Vec<(&str, &str, TabKind)> = vec![
        (
            "Comics",
            "image-x-generic-symbolic",
            TabKind::Comics(pool.clone()),
        ),
        (
            "Series",
            "accessories-dictionary-symbolic",
            TabKind::Volumes(pool.clone()),
        ),
        ("Editoriales", "building-symbolic", TabKind::Publishers),
        (
            "Descargas",
            "folder-download-symbolic",
            TabKind::Downloads(pool.clone()),
        ),
        (
            "ComicVine",
            "web-browser-symbolic",
            TabKind::ComicVineSearch(pool.clone()),
        ),
    ];

    for (label_text, icon, kind) in kinds {
        let row = adw::ActionRow::builder()
            .title(label_text)
            .activatable(true)
            .build();
        row.add_prefix(&gtk::Image::builder().icon_name(icon).pixel_size(20).build());

        let tv = tab_view.clone();
        let popover_ref = popover.downgrade();
        row.connect_activated(move |_| {
            if let Some(pop) = popover_ref.upgrade() {
                pop.popdown();
            }
            let new_page = add_tab(&tv, kind.clone());
            tv.set_selected_page(&new_page);
        });

        list.append(&row);
    }

    popover.set_child(Some(&list));
    popover
}

/// Página temporal con 4 botones grandes para elegir el tipo de tab.
/// Al elegir, inserta la tab real y cierra esta.
fn build_selector_page(tab_view: &adw::TabView, pool: SqlitePool) -> gtk::Widget {
    let center = adw::StatusPage::builder()
        .title("Nueva pestaña")
        .description("Elige qué quieres abrir")
        .icon_name("tab-new-symbolic")
        .build();

    let btn_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .halign(gtk::Align::Center)
        .build();

    let kinds: Vec<(&str, &str, TabKind)> = vec![
        (
            "Comics",
            "image-x-generic-symbolic",
            TabKind::Comics(pool.clone()),
        ),
        (
            "Series",
            "accessories-dictionary-symbolic",
            TabKind::Volumes(pool.clone()),
        ),
        ("Editoriales", "building-symbolic", TabKind::Publishers),
        (
            "Descargas",
            "folder-download-symbolic",
            TabKind::Downloads(pool.clone()),
        ),
        (
            "ComicVine",
            "web-browser-symbolic",
            TabKind::ComicVineSearch(pool.clone()),
        ),
    ];

    for (label_text, icon, kind) in kinds {
        let btn = gtk::Button::new();
        let inner = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .margin_top(16)
            .margin_bottom(16)
            .margin_start(20)
            .margin_end(20)
            .build();
        inner.append(&gtk::Image::builder().icon_name(icon).pixel_size(48).build());
        inner.append(&gtk::Label::new(Some(label_text)));
        btn.set_child(Some(&inner));
        btn.add_css_class("flat");

        let tv = tab_view.clone();
        // Capturamos una referencia débil a `center` — es exactamente el widget
        // que se pasó a tab_view.append(), así tv.page() lo encontrará sin fallo.
        let center_weak = center.downgrade();
        btn.connect_clicked(move |_| {
            let new_page = add_tab(&tv, kind.clone());
            tv.set_selected_page(&new_page);

            if let Some(c) = center_weak.upgrade() {
                let widget: &gtk::Widget = c.upcast_ref();
                tv.close_page(&tv.page(widget));
            }
        });

        btn_box.append(&btn);
    }

    center.set_child(Some(&btn_box));
    center.upcast()
}

// ---------------------------------------------------------------------------
// Menú
// ---------------------------------------------------------------------------

fn build_menu() -> gio::MenuModel {
    let menu = gio::Menu::new();
    menu.append(Some("Escanear directorios"), Some("app.scan"));
    menu.append_section(None, &{
        let section = gio::Menu::new();
        section.append(
            Some("Indexar portadas existentes"),
            Some("win.download-covers"),
        );
        section.append(Some("Generar embeddings CLIP"), Some("win.generate-clip"));
        section.upcast::<gio::MenuModel>()
    });
    menu.append(Some("Preferencias"), Some("app.preferences"));
    menu.append_section(None, &{
        let section = gio::Menu::new();
        section.append(Some("Atajos de teclado"), Some("win.show-help-overlay"));
        section.append(Some("Acerca de Babelcomics"), Some("app.about"));
        section.upcast::<gio::MenuModel>()
    });
    menu.upcast()
}

// ---------------------------------------------------------------------------
// Atajos de teclado
// ---------------------------------------------------------------------------

fn setup_shortcuts(
    win: &adw::ApplicationWindow,
    tab_view: &adw::TabView,
    tab_overview: &adw::TabOverview,
) {
    let controller = gtk::ShortcutController::new();
    controller.set_scope(gtk::ShortcutScope::Managed);

    // Ctrl+W — cerrar tab actual
    {
        let tv = tab_view.clone();
        let action = gtk::CallbackAction::new(move |_, _| {
            if let Some(page) = tv.selected_page() {
                tv.close_page(&page);
            }
            glib::Propagation::Stop
        });
        controller.add_shortcut(gtk::Shortcut::new(
            Some(gtk::ShortcutTrigger::parse_string("<Control>w").unwrap()),
            Some(action),
        ));
    }

    // Ctrl+Tab — siguiente tab
    {
        let tv = tab_view.clone();
        let action = gtk::CallbackAction::new(move |_, _| {
            tv.select_next_page();
            glib::Propagation::Stop
        });
        controller.add_shortcut(gtk::Shortcut::new(
            Some(gtk::ShortcutTrigger::parse_string("<Control>Tab").unwrap()),
            Some(action),
        ));
    }

    // Ctrl+Shift+Tab — pestaña anterior
    {
        let tv = tab_view.clone();
        let action = gtk::CallbackAction::new(move |_, _| {
            tv.select_previous_page();
            glib::Propagation::Stop
        });
        controller.add_shortcut(gtk::Shortcut::new(
            Some(gtk::ShortcutTrigger::parse_string("<Control><Shift>Tab").unwrap()),
            Some(action),
        ));
    }

    // Ctrl+Shift+O — abrir overview de tabs
    {
        let ov = tab_overview.clone();
        let action = gtk::CallbackAction::new(move |_, _| {
            ov.set_open(!ov.is_open());
            glib::Propagation::Stop
        });
        controller.add_shortcut(gtk::Shortcut::new(
            Some(gtk::ShortcutTrigger::parse_string("<Control><Shift>o").unwrap()),
            Some(action),
        ));
    }

    // Escape — cerrar tab actual (útil para volver desde detalles)
    {
        let tv = tab_view.clone();
        let action = gtk::CallbackAction::new(move |_, _| {
            if let Some(page) = tv.selected_page() {
                // Solo cerramos si no es una de las pestañas fijas principales
                // o si el usuario realmente quiere cerrar lo que sea.
                // Por ahora, dejamos que cierre cualquier pestaña para dar fluidez.
                tv.close_page(&page);
            }
            glib::Propagation::Stop
        });
        controller.add_shortcut(gtk::Shortcut::new(
            Some(gtk::ShortcutTrigger::parse_string("Escape").unwrap()),
            Some(action),
        ));
    }

    win.add_controller(controller);
}
