use gtk4::prelude::*;
use gtk4::{self as gtk, gio, glib};
use libadwaita as adw;
use adw::prelude::*;
use sqlx::SqlitePool;

use super::pages;

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
        .icon_name("info-symbolic")
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
        #[weak] tab_view,
        #[upgrade_or_panic]
        move |_| {
            add_tab(&tab_view, TabKind::Selector(pool_for_overview.clone()))
        }
    ));

    win.set_content(Some(&tab_overview));

    // Abrir tabs por defecto
    add_tab(&tab_view, TabKind::Comics(pool.clone()));
    add_tab(&tab_view, TabKind::Volumes(pool.clone()));
    add_tab(&tab_view, TabKind::Publishers);
    add_tab(&tab_view, TabKind::Downloads);

    // Atajos de teclado
    setup_shortcuts(&win, &tab_view, &tab_overview);

    win.maximize();

    win
}

// ---------------------------------------------------------------------------
// Tipos de tab
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub enum TabKind {
    Comics(SqlitePool),
    Volumes(SqlitePool),
    Publishers,
    Downloads,
    ComicVineSearch(SqlitePool),
    /// Página temporal que permite elegir el tipo de tab
    Selector(SqlitePool),
    /// Detalle de un cómic específico
    ComicDetail(i64, SqlitePool),
    /// Detalle de un volumen (serie) específico
    VolumeDetail(i64, SqlitePool),
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

    let (title, icon_name, content): (String, &str, gtk::Widget) = match kind {
        TabKind::Comics(pool) => (
            "Comics".into(),
            "image-x-generic-symbolic",
            pages::comics::build(pool, tab_view.clone()),
        ),
        TabKind::Volumes(pool) => (
            "Series".into(),
            "open-book-symbolic",
            pages::volumes::build(pool, tab_view.clone()),
        ),

        TabKind::Publishers => (
            "Editoriales".into(),
            "building-symbolic",
            pages::publishers::build(),
        ),
        TabKind::Downloads => (
            "Descargas".into(),
            "folder-download-symbolic",
            pages::downloads::build(),
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
        ("Comics",      "image-x-generic-symbolic", TabKind::Comics(pool.clone())),
        ("Series",      "open-book-symbolic",        TabKind::Volumes(pool.clone())),
        ("Editoriales", "building-symbolic",          TabKind::Publishers),
        ("Descargas",   "folder-download-symbolic",  TabKind::Downloads),
        ("ComicVine",   "web-browser-symbolic",      TabKind::ComicVineSearch(pool.clone())),
    ];

    for (label_text, icon, kind) in kinds {
        let row = adw::ActionRow::builder()
            .title(label_text)
            .activatable(true)
            .build();
        row.add_prefix(
            &gtk::Image::builder()
                .icon_name(icon)
                .pixel_size(20)
                .build(),
        );

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
        ("Comics",      "image-x-generic-symbolic", TabKind::Comics(pool.clone())),
        ("Series",      "open-book-symbolic",        TabKind::Volumes(pool.clone())),
        ("Editoriales", "building-symbolic",          TabKind::Publishers),
        ("Descargas",   "folder-download-symbolic",  TabKind::Downloads),
        ("ComicVine",   "web-browser-symbolic",      TabKind::ComicVineSearch(pool.clone())),
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
        controller.add_shortcut(
            gtk::Shortcut::new(
                Some(gtk::ShortcutTrigger::parse_string("<Control>w").unwrap()),
                Some(action),
            )
        );
    }

    // Ctrl+Tab — siguiente tab
    {
        let tv = tab_view.clone();
        let action = gtk::CallbackAction::new(move |_, _| {
            tv.select_next_page();
            glib::Propagation::Stop
        });
        controller.add_shortcut(
            gtk::Shortcut::new(
                Some(gtk::ShortcutTrigger::parse_string("<Control>Tab").unwrap()),
                Some(action),
            )
        );
    }

    // Ctrl+Shift+Tab — pestaña anterior
    {
        let tv = tab_view.clone();
        let action = gtk::CallbackAction::new(move |_, _| {
            tv.select_previous_page();
            glib::Propagation::Stop
        });
        controller.add_shortcut(
            gtk::Shortcut::new(
                Some(gtk::ShortcutTrigger::parse_string("<Control><Shift>Tab").unwrap()),
                Some(action),
            )
        );
    }

    // Ctrl+Shift+O — abrir overview de tabs
    {
        let ov = tab_overview.clone();
        let action = gtk::CallbackAction::new(move |_, _| {
            ov.set_open(!ov.is_open());
            glib::Propagation::Stop
        });
        controller.add_shortcut(
            gtk::Shortcut::new(
                Some(gtk::ShortcutTrigger::parse_string("<Control><Shift>o").unwrap()),
                Some(action),
            )
        );
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
        controller.add_shortcut(
            gtk::Shortcut::new(
                Some(gtk::ShortcutTrigger::parse_string("Escape").unwrap()),
                Some(action),
            )
        );
    }

    win.add_controller(controller);
}
