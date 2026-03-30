use std::path::Path;

use gtk4::prelude::*;
use gtk4::{self as gtk, gio, glib};
use libadwaita as adw;
use adw::prelude::*;
use sqlx::SqlitePool;

use crate::helpers::paths::comic_thumbnail_path;
use crate::helpers::suggestion_service::SuggestionResult;
use crate::helpers::thumbnail::CardSize;
use crate::models::{ComicbookView, NewComicbookDetail};
use crate::repositories::{ComicbookDetailRepository, ComicbookRepository, SetupRepository};
use crate::ui::run_in_background;

/// Una vez creada la `adw::TabPage`, actualiza su título con el nombre real del cómic.
pub fn setup_tab_title(comic_id: i64, pool: SqlitePool, page: glib::WeakRef<adw::TabPage>) {
    run_in_background(
        tokio::runtime::Handle::current(),
        async move {
            ComicbookRepository::new(&pool)
                .get_view_by_id(comic_id)
                .await
                .ok()
                .flatten()
        },
        move |comic_opt| {
            let Some(page) = page.upgrade() else { return };
            let Some(comic) = comic_opt else { return };
            let filename = Path::new(&comic.path)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| comic.path.clone());
            let display = comic.titulo.unwrap_or(filename);
            page.set_title(&display);
        },
    );
}

/// Construye el widget de detalle de un cómic.
/// Se usa como contenido de una `adw::TabPage`.
pub fn build(comic_id: i64, pool: SqlitePool) -> gtk::Widget {
    // Sub-TabView interno: Información | Páginas
    let inner_tab_view = adw::TabView::new();
    inner_tab_view.set_vexpand(true);

    let inner_tab_bar = adw::TabBar::new();
    inner_tab_bar.set_view(Some(&inner_tab_view));
    inner_tab_bar.set_autohide(false);

    let content_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    content_box.append(&inner_tab_bar);
    content_box.append(&inner_tab_view);

    // Pestaña: Información
    let info_content = build_info_tab(comic_id, pool.clone());
    let info_page = inner_tab_view.append(&info_content);
    info_page.set_title("Información");
    info_page.set_icon(Some(&gio::ThemedIcon::new("info-outline-symbolic")));

    // Pestaña: Páginas
    let pages_content = build_pages_tab(comic_id, pool.clone());
    let pages_page = inner_tab_view.append(&pages_content);
    pages_page.set_title("Páginas");
    pages_page.set_icon(Some(&gio::ThemedIcon::new("view-grid-symbolic")));

    content_box.upcast()
}

// ── Pestaña Información ────────────────────────────────────────────────────────

fn build_info_tab(comic_id: i64, pool: SqlitePool) -> gtk::Widget {
    let stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .vexpand(true)
        .build();

    let spinner = adw::Spinner::builder()
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    stack.add_named(&spinner, Some("loading"));

    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .vexpand(true)
        .build();
    stack.add_named(&scroll, Some("content"));

    let error_page = adw::StatusPage::builder()
        .title("No encontrado")
        .description("No se pudo cargar la información del cómic")
        .icon_name("dialog-error-symbolic")
        .build();
    stack.add_named(&error_page, Some("error"));

    stack.set_visible_child_name("loading");

    let stack_done = stack.clone();
    let scroll_done = scroll.clone();
    let pool_done = pool.clone();

    run_in_background(
        tokio::runtime::Handle::current(),
        async move {
            let comic = ComicbookRepository::new(&pool)
                .get_view_by_id(comic_id)
                .await
                .ok()
                .flatten()?;
            let setup = SetupRepository::new(&pool)
                .get()
                .await
                .unwrap_or_default();
            let card_size = CardSize::from_db(setup.thumbnail_size);
            Some((comic, card_size))
        },
        move |res| match res {
            None => stack_done.set_visible_child_name("error"),
            Some((comic, card_size)) => {
                let content = build_info_content(&comic, card_size, pool_done.clone());
                scroll_done.set_child(Some(&content));
                stack_done.set_visible_child_name("content");
            }
        },
    );

    stack.upcast()
}

fn build_info_content(comic: &ComicbookView, card_size: CardSize, pool: SqlitePool) -> gtk::Widget {
    let main_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(20)
        .margin_top(20)
        .margin_bottom(20)
        .margin_start(20)
        .margin_end(20)
        .build();

    // ── Header: portada + datos básicos ───────────────────────────────────────
    let header_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(20)
        .build();

    header_box.append(&build_cover(comic, card_size));
    header_box.append(&build_basic_info(comic, pool));
    main_box.append(&header_box);

    // ── Catalogación (si está clasificado) ────────────────────────────────────
    if comic.titulo.is_some() || comic.nombre_volume.is_some() {
        main_box.append(&build_catalog_group(comic));
    }

    main_box.upcast()
}

fn build_cover(comic: &ComicbookView, card_size: CardSize) -> gtk::Widget {
    let (cw, ch) = card_size.dims();

    let image_container = gtk::Box::builder()
        .height_request(ch as i32)
        .hexpand(false)
        .css_classes(["comic-cover-container", "card"])
        .valign(gtk::Align::Start)
        .build();

    let placeholder = gtk::Image::builder()
        .icon_name("image-x-generic-symbolic")
        .pixel_size((cw / 3) as i32)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .opacity(0.3)
        .build();
    image_container.append(&placeholder);

    let id_comicbook = comic.id_comicbook;
    let ruta_cover = comic.ruta_cover.clone();
    let path = comic.path.clone();
    let container_weak = image_container.downgrade();

    run_in_background(
        tokio::runtime::Handle::current(),
        async move {
            // 1. Portada descargada de ComicVine
            if let Some(ref ruta) = ruta_cover {
                if let Ok(bytes) = tokio::fs::read(ruta).await {
                    return Some(bytes);
                }
            }

            // 2. Thumbnail generado por la app — generamos on-demand si no existe
            let thumb_path = comic_thumbnail_path(id_comicbook, card_size);
            if !thumb_path.exists() {
                let path_clone = path.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    if let Ok(bytes) = crate::helpers::extractor::extract_cover(&path_clone) {
                        let _ = crate::helpers::thumbnail::generate_all_thumbnails(&bytes, id_comicbook);
                    }
                }).await;
            }
            tokio::fs::read(&thumb_path).await.ok()
        },
        move |bytes_opt| {
            if let (Some(bytes), Some(container)) = (bytes_opt, container_weak.upgrade()) {
                let gbytes = glib::Bytes::from_owned(bytes);
                if let Ok(texture) = gtk::gdk::Texture::from_bytes(&gbytes) {
                    while let Some(child) = container.first_child() {
                        container.remove(&child);
                    }
                    let picture = gtk::Picture::for_paintable(&texture);
                    picture.set_content_fit(gtk::ContentFit::Contain);
                    picture.set_can_shrink(true);
                    picture.add_css_class("cover-image");
                    container.append(&picture);
                }
            }
        },
    );

    image_container.upcast()
}

fn build_basic_info(comic: &ComicbookView, pool: SqlitePool) -> gtk::Box {
    let info_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(16)
        .hexpand(true)
        .valign(gtk::Align::Start)
        .build();

    // Nombre del archivo como título
    let filename = Path::new(&comic.path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| comic.path.clone());

    let title_label = gtk::Label::builder()
        .label(&filename)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .halign(gtk::Align::Start)
        .selectable(true)
        .css_classes(["title-1"])
        .build();
    info_box.append(&title_label);

    // Grupo de info del archivo
    let file_group = adw::PreferencesGroup::new();
    file_group.set_title("Archivo");

    add_row(&file_group, "ID", &format!("#{}", comic.id_comicbook));

    let estado = if comic.titulo.is_some() { "Clasificado" } else { "Sin clasificar" };
    add_row(&file_group, "Estado", estado);

    if let Some(ref calidad) = comic.calidad {
        add_row(&file_group, "Calidad", calidad);
    }

    add_row(&file_group, "En papelera", if comic.en_papelera { "Sí" } else { "No" });

    // Ruta
    let path_row = adw::ActionRow::builder()
        .title("Ruta")
        .subtitle(&comic.path)
        .subtitle_selectable(true)
        .build();
    file_group.add(&path_row);

    if let Some(ref err) = comic.error_ultimo_escaneo {
        let err_row = adw::ActionRow::builder()
            .title("Error de escaneo")
            .subtitle(err)
            .subtitle_selectable(true)
            .build();
        file_group.add(&err_row);
    }

    info_box.append(&file_group);

    // Botones de acción
    info_box.append(&build_action_buttons(comic, pool));

    info_box
}

fn build_catalog_group(comic: &ComicbookView) -> gtk::Widget {
    let group = adw::PreferencesGroup::new();
    group.set_title("Catalogación");

    if let Some(ref titulo) = comic.titulo {
        add_row(&group, "Título", titulo);
    }
    if let Some(ref numero) = comic.numero {
        add_row(&group, "Número", numero);
    }
    if let Some(ref volumen) = comic.nombre_volume {
        add_row(&group, "Volumen", volumen);
    }
    if let Some(ref editorial) = comic.nombre_publisher {
        add_row(&group, "Editorial", editorial);
    }
    if let Some(calificacion) = comic.calificacion {
        let stars = "★".repeat((calificacion.round() as usize).clamp(0, 5));
        add_row(&group, "Calificación", &stars);
    }

    group.upcast()
}

fn build_action_buttons(comic: &ComicbookView, pool: SqlitePool) -> gtk::Widget {
    let action_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .margin_top(8)
        .build();

    // Botón LEER (Lanzar lector independiente)
    let path_for_reader = comic.path.clone();
    let read_btn = gtk::Button::builder()
        .label("Leer")
        .icon_name("book-open-symbolic")
        .css_classes(["suggested-action"])
        .tooltip_text("Abrir en el lector integrado")
        .build();
    read_btn.connect_clicked(move |_| {
        let Some(app) = gio::Application::default() else { return };
        if let Some(adw_app) = app.downcast_ref::<adw::Application>() {
            crate::ui::reader::ReaderWindow::open(adw_app, &path_for_reader);
        }
    });
    action_box.append(&read_btn);

    let path_for_file = comic.path.clone();
    let open_file_btn = gtk::Button::builder()
        .label("Abrir archivo")
        .icon_name("document-open-symbolic")
        .tooltip_text("Abrir con la aplicación predeterminada del sistema")
        .build();
    open_file_btn.connect_clicked(move |_| {
        let _ = std::process::Command::new("xdg-open")
            .arg(&path_for_file)
            .spawn();
    });
    action_box.append(&open_file_btn);

    let folder_path = Path::new(&comic.path)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let open_folder_btn = gtk::Button::builder()
        .label("Abrir carpeta")
        .icon_name("folder-open-symbolic")
        .tooltip_text("Mostrar en el explorador de archivos")
        .build();
    open_folder_btn.connect_clicked(move |_| {
        let _ = std::process::Command::new("xdg-open")
            .arg(&folder_path)
            .spawn();
    });
    action_box.append(&open_folder_btn);

    // Buscar / cambiar catalogación por similitud de portada
    let suggest_label = if comic.titulo.is_none() {
        "Buscar candidatos"
    } else {
        "Cambiar catalogación"
    };
    let suggest_btn = gtk::Button::builder()
        .label(suggest_label)
        .icon_name("system-search-symbolic")
        .tooltip_text("Buscar series similares por portada")
        .build();

    let path_s = comic.path.clone();
    let id_s = comic.id_comicbook;
    let pool_s = pool.clone();
    suggest_btn.connect_clicked(move |btn| {
        let parent = btn.root().and_then(|r| r.downcast::<gtk::Window>().ok());
        show_suggest_dialog(id_s, path_s.clone(), pool_s.clone(), parent.as_ref());
    });
    action_box.append(&suggest_btn);

    action_box.upcast()
}

// ── Diálogo de sugerencia de catalogación ─────────────────────────────────────

fn show_suggest_dialog(
    comic_id: i64,
    comic_path: String,
    pool: SqlitePool,
    parent: Option<&gtk::Window>,
) {
    let dialog = adw::Window::builder()
        .title("Candidatos de catalogación")
        .modal(true)
        .default_width(720)
        .default_height(540)
        .destroy_with_parent(true)
        .build();
    if let Some(p) = parent {
        dialog.set_transient_for(Some(p));
    }

    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    toolbar.add_top_bar(&header);

    let stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .vexpand(true)
        .build();

    // Pantalla de carga
    let loading_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    loading_box.append(
        &adw::Spinner::builder()
            .halign(gtk::Align::Center)
            .build(),
    );
    loading_box.append(
        &gtk::Label::builder()
            .label("Calculando similitudes…")
            .css_classes(["dim-label"])
            .build(),
    );
    stack.add_named(&loading_box, Some("loading"));

    stack.add_named(
        &adw::StatusPage::builder()
            .title("Sin candidatos")
            .description("No se encontraron portadas similares en la biblioteca")
            .icon_name("system-search-symbolic")
            .build(),
        Some("empty"),
    );

    stack.add_named(
        &adw::StatusPage::builder()
            .title("Error")
            .description("No se pudo procesar la portada del archivo")
            .icon_name("dialog-error-symbolic")
            .build(),
        Some("error"),
    );

    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();
    let results_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    scroll.set_child(Some(&results_list));
    stack.add_named(&scroll, Some("results"));
    stack.set_visible_child_name("loading");

    toolbar.set_content(Some(&stack));
    dialog.set_content(Some(&toolbar));
    dialog.present();

    let stack_done = stack.clone();
    let list_done = results_list.clone();
    let pool_done = pool.clone();
    let dialog_weak: glib::WeakRef<adw::Window> = dialog.downgrade();

    run_in_background(
        tokio::runtime::Handle::current(),
        async move {
            crate::helpers::suggestion_service::suggest_for_comic(&pool_done, comic_id, 10).await
        },
        move |result| match result {
            Err(_) => {
                stack_done.set_visible_child_name("error");
            }
            Ok(candidates) if candidates.is_empty() => {
                stack_done.set_visible_child_name("empty");
            }
            Ok(candidates) => {
                for candidate in candidates {
                    let row =
                        build_candidate_row(&candidate, comic_id, pool.clone(), dialog_weak.clone());
                    list_done.append(&row);
                }
                stack_done.set_visible_child_name("results");
            }
        },
    );
}

fn build_candidate_row(
    candidate: &SuggestionResult,
    comic_id: i64,
    pool: SqlitePool,
    dialog_weak: glib::WeakRef<adw::Window>,
) -> gtk::ListBoxRow {
    let row_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .margin_top(10)
        .margin_bottom(10)
        .margin_start(10)
        .margin_end(10)
        .build();

    // Miniatura de portada
    let cover_box = gtk::Box::builder()
        .width_request(44)
        .height_request(64)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .css_classes(["comic-cover-container"])
        .build();
    cover_box.append(
        &gtk::Image::builder()
            .icon_name("open-book-symbolic")
            .pixel_size(20)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .opacity(0.4)
            .build(),
    );

    if let Some(ruta) = candidate.ruta_cover.clone() {
        let cover_weak = cover_box.downgrade();
        run_in_background(
            tokio::runtime::Handle::current(),
            async move {
                let bytes = tokio::fs::read(&ruta).await.ok()?;
                tokio::task::spawn_blocking(move || -> Option<Vec<u8>> {
                    let img = image::load_from_memory(&bytes).ok()?;
                    let scaled = img.resize(u32::MAX, 64, image::imageops::FilterType::Triangle);
                    let mut out = Vec::new();
                    scaled
                        .write_to(
                            &mut std::io::Cursor::new(&mut out),
                            image::ImageFormat::Jpeg,
                        )
                        .ok()?;
                    Some(out)
                })
                .await
                .ok()?
            },
            move |bytes_opt| {
                if let (Some(bytes), Some(container)) = (bytes_opt, cover_weak.upgrade()) {
                    let gbytes = glib::Bytes::from_owned(bytes);
                    if let Ok(texture) = gtk::gdk::Texture::from_bytes(&gbytes) {
                        while let Some(child) = container.first_child() {
                            container.remove(&child);
                        }
                        let pic = gtk::Picture::for_paintable(&texture);
                        pic.set_content_fit(gtk::ContentFit::Contain);
                        container.append(&pic);
                    }
                }
            },
        );
    }
    row_box.append(&cover_box);

    // Info de la serie / número
    let info_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(3)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .build();

    info_box.append(
        &gtk::Label::builder()
            .label(
                candidate
                    .nombre_volume
                    .as_deref()
                    .unwrap_or("Serie desconocida"),
            )
            .halign(gtk::Align::Start)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["heading"])
            .build(),
    );

    let issue_text = match &candidate.numero {
        Some(n) => format!("#{} — {}", n, candidate.titulo),
        None => candidate.titulo.clone(),
    };
    info_box.append(
        &gtk::Label::builder()
            .label(&issue_text)
            .halign(gtk::Align::Start)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["caption"])
            .build(),
    );

    info_box.append(
        &gtk::Label::builder()
            .label(&similarity_label(candidate.distance))
            .halign(gtk::Align::Start)
            .css_classes(["caption", "dim-label"])
            .build(),
    );
    row_box.append(&info_box);

    // Botón catalogar
    let catalog_btn = gtk::Button::builder()
        .label("Catalogar")
        .css_classes(["suggested-action"])
        .valign(gtk::Align::Center)
        .build();

    let info_id = candidate.id_comicbook_info;
    catalog_btn.connect_clicked(move |_| {
        let pool_c = pool.clone();
        tokio::runtime::Handle::current().spawn(async move {
            let _ = ComicbookRepository::new(&pool_c)
                .set_info(comic_id, Some(info_id))
                .await;
        });
        if let Some(dialog) = dialog_weak.upgrade() {
            dialog.close();
        }
    });
    row_box.append(&catalog_btn);

    gtk::ListBoxRow::builder().child(&row_box).build()
}

fn similarity_label(distance: u32) -> String {
    match distance {
        0..=5 => "Coincidencia exacta".to_string(),
        6..=15 => "Muy similar".to_string(),
        16..=25 => "Similar".to_string(),
        26..=35 => "Posible coincidencia".to_string(),
        _ => format!("Similitud baja (distancia {})", distance),
    }
}

fn add_row(group: &adw::PreferencesGroup, title: &str, subtitle: &str) {
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(subtitle)
        .subtitle_selectable(true)
        .build();
    group.add(&row);
}

// ── Pestaña Páginas ───────────────────────────────────────────────────────────

fn build_pages_tab(comic_id: i64, pool: SqlitePool) -> gtk::Widget {
    let stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .vexpand(true)
        .build();

    let spinner = adw::Spinner::builder()
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    stack.add_named(&spinner, Some("loading"));

    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();

    let wrap_box = adw::WrapBox::builder()
        .valign(gtk::Align::Start)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .child_spacing(12)
        .line_spacing(12)
        .build();
    
    scroll.set_child(Some(&wrap_box));
    stack.add_named(&scroll, Some("content"));

    stack.set_visible_child_name("loading");

    let stack_done = stack.clone();
    let wrap_done = wrap_box.clone();
    let pool_task = pool.clone();

    run_in_background(
        tokio::runtime::Handle::current(),
        async move {
            let comic = ComicbookRepository::new(&pool_task)
                .get_view_by_id(comic_id).await.ok().flatten()?;
            let card_size = CardSize::from_db(
                SetupRepository::new(&pool_task).get().await.unwrap_or_default().thumbnail_size
            );

            // Extraer páginas al directorio temporal
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            use std::hash::{Hash, Hasher};
            comic.path.hash(&mut hasher);
            let temp_dir = std::env::temp_dir()
                .join(format!("babelcomics_detail_{:x}", hasher.finish()));

            let pages = crate::helpers::extractor::extract_all_pages(&comic.path, &temp_dir).ok()?;

            // Poblar comicbooks_detail si aún no tiene entradas para este comic
            let detail_repo = ComicbookDetailRepository::new(&pool_task);
            let existing = detail_repo.get_by_comicbook(comic_id).await.unwrap_or_default();
            if existing.is_empty() {
                let page_names = crate::helpers::extractor::list_pages(&comic.path).unwrap_or_default();
                let new_pages: Vec<NewComicbookDetail> = page_names
                    .into_iter()
                    .enumerate()
                    .map(|(i, name)| NewComicbookDetail {
                        comicbook_id:  comic_id,
                        indice_pagina: i as i64,
                        orden_pagina:  i as i64,
                        tipo_pagina:   crate::models::TipoPagina::Story,
                        nombre_pagina: Some(name),
                    })
                    .collect();
                let _ = detail_repo.upsert_all(&new_pages).await;
            }

            // Índice de la página marcada como FrontCover (si existe)
            let cover_indice = detail_repo
                .get_by_comicbook(comic_id).await.unwrap_or_default()
                .into_iter()
                .find(|p| p.tipo_pagina == crate::models::TipoPagina::FrontCover.to_db())
                .map(|p| p.indice_pagina);

            Some((pages, temp_dir, card_size, cover_indice))
        },
        move |res| {
            if let Some((pages, _temp_dir, card_size, cover_indice)) = res {
                use std::rc::Rc;
                use std::cell::RefCell;
                // Botones de cover compartidos entre todas las cards para poder actualizarlos
                let cover_btns: Rc<RefCell<Vec<(i64, gtk::Button)>>> =
                    Rc::new(RefCell::new(Vec::new()));

                for (i, path) in pages.into_iter().enumerate() {
                    let is_cover = cover_indice.map_or(i == 0, |ci| ci == i as i64);
                    let (card, btn) = build_page_card(
                        i, path, card_size, comic_id, pool.clone(), is_cover, cover_btns.clone(),
                    );
                    cover_btns.borrow_mut().push((i as i64, btn));
                    wrap_done.append(&card);
                }
                stack_done.set_visible_child_name("content");
            }
        },
    );

    stack.upcast()
}

fn build_page_card(
    index: usize,
    path: std::path::PathBuf,
    card_size: CardSize,
    comicbook_id: i64,
    pool: SqlitePool,
    is_cover: bool,
    cover_btns: std::rc::Rc<std::cell::RefCell<Vec<(i64, gtk::Button)>>>,
) -> (gtk::Widget, gtk::Button) {
    let (cw, ch) = card_size.dims();

    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(0)
        .margin_top(4)
        .margin_bottom(4)
        .margin_start(4)
        .margin_end(4)
        .hexpand(false)
        .halign(gtk::Align::Center)
        .css_classes(["card"])
        .build();

    let image_container = gtk::Box::builder()
        .height_request(ch as i32)
        .hexpand(false)
        .halign(gtk::Align::Center)
        .overflow(gtk::Overflow::Hidden)
        .css_classes(["comic-cover-container"])
        .build();

    let placeholder = gtk::Image::builder()
        .icon_name("image-x-generic-symbolic")
        .pixel_size((cw / 3) as i32)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .opacity(0.3)
        .build();
    image_container.append(&placeholder);

    let btn_icon = if is_cover { "starred-symbolic" } else { "emblem-default-symbolic" };
    let btn_cover = gtk::Button::builder()
        .icon_name(btn_icon)
        .halign(gtk::Align::End)
        .valign(gtk::Align::Start)
        .margin_top(4)
        .margin_end(4)
        .tooltip_text("Usar como portada")
        .css_classes(["osd", "circular"])
        .build();

    let pool_btn = pool.clone();
    let page_path = path.clone();
    let cover_btns_click = cover_btns.clone();
    btn_cover.connect_clicked(move |_| {
        let indice = index as i64;

        // Actualizar iconos de todos los botones en el hilo GTK
        for (idx, btn) in cover_btns_click.borrow().iter() {
            btn.set_icon_name(if *idx == indice {
                "starred-symbolic"
            } else {
                "emblem-default-symbolic"
            });
        }

        // Persistir en BD y regenerar thumbnail en background
        let pool_c = pool_btn.clone();
        let path_c = page_path.clone();
        tokio::runtime::Handle::current().spawn(async move {
            let _ = ComicbookDetailRepository::new(&pool_c)
                .set_as_cover(comicbook_id, indice)
                .await;

            let _ = tokio::task::spawn_blocking(move || {
                if let Ok(bytes) = std::fs::read(&path_c) {
                    let _ = crate::helpers::thumbnail::generate_all_thumbnails(&bytes, comicbook_id);
                }
            }).await;
        });
    });

    let overlay = gtk::Overlay::builder()
        .child(&image_container)
        .hexpand(false)
        .build();
    overlay.add_overlay(&btn_cover);

    let lbl = gtk::Label::builder()
        .label(&format!("Página {}", index + 1))
        .css_classes(["caption", "dim-label"])
        .margin_top(6)
        .margin_bottom(6)
        .halign(gtk::Align::Center)
        .build();

    card.append(&overlay);
    card.append(&lbl);

    let container_weak = image_container.downgrade();
    run_in_background(
        tokio::runtime::Handle::current(),
        async move {
            let bytes = tokio::fs::read(&path).await.ok()?;
            tokio::task::spawn_blocking(move || -> Option<Vec<u8>> {
                let img = image::load_from_memory(&bytes).ok()?;
                let scaled = img.resize(u32::MAX, ch, image::imageops::FilterType::Triangle);
                let mut out = Vec::new();
                scaled.write_to(
                    &mut std::io::Cursor::new(&mut out),
                    image::ImageFormat::Jpeg,
                ).ok()?;
                Some(out)
            }).await.ok()?
        },
        move |bytes_opt| {
            if let (Some(bytes), Some(container)) = (bytes_opt, container_weak.upgrade()) {
                let gbytes = glib::Bytes::from_owned(bytes);
                if let Ok(texture) = gtk::gdk::Texture::from_bytes(&gbytes) {
                    while let Some(child) = container.first_child() {
                        container.remove(&child);
                    }
                    let picture = gtk::Picture::for_paintable(&texture);
                    picture.set_content_fit(gtk::ContentFit::Contain);
                    picture.add_css_class("cover-image");
                    container.append(&picture);
                }
            }
        },
    );

    (card.upcast(), btn_cover)
}
