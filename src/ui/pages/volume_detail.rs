use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{self as gtk, gio};
use libadwaita as adw;
use adw::prelude::*;
use sqlx::SqlitePool;

use crate::models::{Volume, ComicbookInfoView};
use crate::repositories::{VolumeRepository, ComicbookInfoRepository, PublisherRepository};
use crate::ui::run_in_background;

/// Construye el widget de detalle de un volumen.
pub fn build(volume_id: i64, pool: SqlitePool) -> gtk::Widget {
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

    // Necesitamos el nombre del volumen para las rutas de los thumbnails de los issues
    let pool_v = pool.clone();
    let pool_v2 = pool.clone();
    let tab_view_v = inner_tab_view.clone();
    run_in_background(
        tokio::runtime::Handle::current(),
        async move {
            VolumeRepository::new(&pool_v).get_by_id(volume_id).await.ok().flatten()
        },
        move |vol_opt| {
            let name = vol_opt.map(|v| v.nombre).unwrap_or_default();

            // Pestaña: Información
            let info_content = build_info_tab(volume_id, pool_v2.clone());
            let info_page = tab_view_v.append(&info_content);
            info_page.set_title("Información");
            info_page.set_icon(Some(&gio::ThemedIcon::new("info-outline-symbolic")));

            // Pestaña: Issues
            let issues_content = build_issues_tab(volume_id, &name, pool_v2);
            let issues_page = tab_view_v.append(&issues_content);
            issues_page.set_title("Issues");
            issues_page.set_icon(Some(&gio::ThemedIcon::new("view-list-symbolic")));
        }
    );

    content_box.upcast()
}

// ── Pestaña Información ────────────────────────────────────────────────────────

fn build_info_tab(volume_id: i64, pool: SqlitePool) -> gtk::Widget {
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
    stack.add_named(&scroll, Some("content"));

    stack.set_visible_child_name("loading");

    let stack_done = stack.clone();
    let scroll_done = scroll.clone();
    let pool_task = pool.clone();

    run_in_background(
        tokio::runtime::Handle::current(),
        async move {
            let vol = VolumeRepository::new(&pool_task).get_by_id(volume_id).await.ok().flatten()?;
            let publ = PublisherRepository::new(&pool_task).get_by_id(vol.id_publisher).await.ok().flatten();
            let issues = ComicbookInfoRepository::new(&pool_task).get_view_by_volume(volume_id).await.unwrap_or_default();
            
            let owned = issues.iter().filter(|i| i.physical_count > 0).count();
            let total = vol.cantidad_numeros;
            let percent = if total > 0 { (owned as f64 / total as f64) * 100.0 } else { 0.0 };
            
            let first_issue_cover = issues.iter()
                .filter_map(|i| i.ruta_cover.as_ref())
                .next()
                .cloned();

            Some((vol, publ, owned, total, percent, first_issue_cover))
        },
        move |res| {
            if let Some((vol, publ, owned, total, percent, first_issue_cover)) = res {
                let content = build_info_content(&vol, publ.as_ref(), owned, total, percent, first_issue_cover);
                scroll_done.set_child(Some(&content));
                stack_done.set_visible_child_name("content");
            } else {
                stack_done.set_visible_child_name("loading");
            }
        },
    );

    stack.upcast()
}

fn build_info_content(
    vol: &Volume,
    publisher: Option<&crate::models::Publisher>,
    owned: usize,
    total: i64,
    percent: f64,
    first_issue_cover: Option<String>
) -> gtk::Widget {
    let main_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(24)
        .margin_top(24)
        .margin_bottom(24)
        .margin_start(24)
        .margin_end(24)
        .build();

    // --- Cabecera: Portada + Datos ---
    let header = gtk::Box::builder().spacing(24).build();
    
    let cover = gtk::Picture::builder()
        .width_request(200)
        .height_request(280)
        .content_fit(gtk::ContentFit::Contain)
        .css_classes(["card"])
        .build();
    
    let volume_thumb = crate::helpers::paths::volume_thumbnail_path(vol.id_volume);
    if volume_thumb.exists() {
        cover.set_filename(Some(volume_thumb));
    } else if let Some(path) = first_issue_cover {
        cover.set_filename(Some(std::path::PathBuf::from(path)));
    }
    
    header.append(&cover);

    let info_side = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .hexpand(true)
        .build();
    
    info_side.append(&gtk::Label::builder()
        .label(&vol.nombre)
        .halign(gtk::Align::Start)
        .css_classes(["title-1"])
        .wrap(true)
        .build());

    let stats_group = adw::PreferencesGroup::builder().title("Estadísticas de colección").build();
    
    let id_row = adw::ActionRow::builder()
        .title("ID del Volumen")
        .subtitle(vol.id_volume.to_string())
        .build();
    stats_group.add(&id_row);

    if let Some(cv_id) = vol.id_comicvine {
        let cv_row = adw::ActionRow::builder()
            .title("ID ComicVine")
            .subtitle(cv_id.to_string())
            .build();
        stats_group.add(&cv_row);
    }

    let completion_row = adw::ActionRow::builder()
        .title("Completitud")
        .subtitle(format!("{}/{} cómics ({:.1}%)", owned, total, percent))
        .build();
    stats_group.add(&completion_row);

    let year_row = adw::ActionRow::builder()
        .title("Año de inicio")
        .subtitle(vol.anio_inicio.to_string())
        .build();
    stats_group.add(&year_row);

    if let Some(p) = publisher {
        let pub_row = adw::ActionRow::builder()
            .title("Editorial")
            .subtitle(&p.nombre)
            .build();
        stats_group.add(&pub_row);
    }

    info_side.append(&stats_group);
    header.append(&info_side);
    main_box.append(&header);

    // --- Resumen ---
    if !vol.deck.is_empty() {
        let deck_group = adw::PreferencesGroup::builder().title("Resumen").build();
        let deck_label = gtk::Label::builder()
            .label(&vol.deck)
            .wrap(true)
            .halign(gtk::Align::Start)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        deck_group.add(&deck_label);
        main_box.append(&deck_group);
    }

    main_box.upcast()
}

// ── Pestaña Issues ─────────────────────────────────────────────────────────────

fn build_issues_tab(volume_id: i64, volume_name: &str, pool: SqlitePool) -> gtk::Widget {
    let main_vbox = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(0)
        .build();

    // ── Barra de herramientas: búsqueda + filtros ─────────────────────────────
    let search_entry = gtk::SearchEntry::builder()
        .placeholder_text("Filtrar por número o título…")
        .hexpand(true)
        .build();

    let check_poseidos = gtk::CheckButton::with_label("Solo poseídos");

    let toolbar_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(12)
        .margin_end(12)
        .build();
    toolbar_box.append(&search_entry);
    toolbar_box.append(&check_poseidos);
    main_vbox.append(&toolbar_box);

    // ── Grid ─────────────────────────────────────────────────────────────────
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();

    let wrap_box = adw::WrapBox::builder()
        .valign(gtk::Align::Start)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(8)
        .margin_end(8)
        .child_spacing(8)
        .line_spacing(8)
        .build();

    scroll.set_child(Some(&wrap_box));
    main_vbox.append(&scroll);

    // ── Estado de paginación ──────────────────────────────────────────────────
    let offset:     Rc<Cell<i64>>  = Rc::new(Cell::new(0));
    let loading:    Rc<Cell<bool>> = Rc::new(Cell::new(false));
    let all_loaded: Rc<Cell<bool>> = Rc::new(Cell::new(false));
    let query:      Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let poseidos:   Rc<Cell<bool>> = Rc::new(Cell::new(false));
    let v_name = Rc::new(volume_name.to_string());

    const PAGE: i64 = 50;

    // Función de recarga (cierra sobre todo el estado)
    let make_loader = {
        let pool = pool.clone();
        let wrap_box = wrap_box.clone();
        let offset = offset.clone();
        let loading = loading.clone();
        let all_loaded = all_loaded.clone();
        let query = query.clone();
        let poseidos = poseidos.clone();
        let v_name = v_name.clone();

        move || {
            if loading.get() { return; }
            loading.set(true);

            let pool_t   = pool.clone();
            let wb       = wrap_box.clone();
            let off      = offset.clone();
            let load     = loading.clone();
            let done     = all_loaded.clone();
            let q        = query.borrow().clone();
            let solo     = poseidos.get();
            let cur_off  = offset.get();
            let v_name_t = v_name.clone();

            run_in_background(
                tokio::runtime::Handle::current(),
                async move {
                    let setup = crate::repositories::SetupRepository::new(&pool_t)
                        .get().await.unwrap_or_default();
                    let card_size = crate::helpers::thumbnail::CardSize::from_db(setup.thumbnail_size);
                    let issues = ComicbookInfoRepository::new(&pool_t)
                        .get_view_by_volume_page(volume_id, PAGE, cur_off, q.as_deref(), solo)
                        .await
                        .unwrap_or_default();
                    (issues, card_size)
                },
                move |(issues, card_size)| {
                    load.set(false);
                    let received = issues.len() as i64;
                    if received < PAGE { done.set(true); }
                    off.set(cur_off + received);

                    let issues = Rc::new(issues);
                    let idx    = Rc::new(Cell::new(0usize));
                    const BATCH: usize = 20;

                    glib::idle_add_local(move || {
                        let start = idx.get();
                        let end   = (start + BATCH).min(issues.len());
                        for issue in &issues[start..end] {
                            wb.append(&build_issue_card(issue, &v_name_t, card_size));
                        }
                        idx.set(end);
                        if end >= issues.len() {
                            glib::ControlFlow::Break
                        } else {
                            glib::ControlFlow::Continue
                        }
                    });
                },
            );
        }
    };

    let loader = Rc::new(make_loader);

    // ── Reset + primera carga ─────────────────────────────────────────────────
    let reset = {
        let wrap_box  = wrap_box.clone();
        let offset    = offset.clone();
        let loading   = loading.clone();
        let all_loaded = all_loaded.clone();
        let loader    = loader.clone();
        move || {
            while let Some(c) = wrap_box.first_child() { wrap_box.remove(&c); }
            offset.set(0);
            loading.set(false);
            all_loaded.set(false);
            loader();
        }
    };
    let reset = Rc::new(reset);

    // Búsqueda
    {
        let query   = query.clone();
        let se      = search_entry.clone();
        let reset_s = reset.clone();
        search_entry.connect_search_changed(move |_| {
            let text = se.text().to_string();
            *query.borrow_mut() = if text.is_empty() { None } else { Some(text) };
            reset_s();
        });
    }

    // Filtro poseídos
    {
        let poseidos = poseidos.clone();
        let reset_p  = reset.clone();
        check_poseidos.connect_toggled(move |btn| {
            poseidos.set(btn.is_active());
            reset_p();
        });
    }

    // Scroll infinito
    {
        let adj = scroll.vadjustment();
        let loader_s   = loader.clone();
        let loading_s  = loading.clone();
        let all_done_s = all_loaded.clone();
        adj.connect_value_changed(move |adj| {
            if loading_s.get() || all_done_s.get() { return; }
            if adj.value() + adj.page_size() >= adj.upper() - 400.0 {
                loader_s();
            }
        });
    }

    // Carga inicial
    loader();

    main_vbox.upcast()
}

fn build_issue_card(view: &ComicbookInfoView, volume_name: &str, card_size: crate::helpers::thumbnail::CardSize) -> gtk::Widget {
    let (cw, ch) = card_size.dims();

    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(0)
        .margin_top(4)
        .margin_bottom(4)
        .halign(gtk::Align::Center)
        .hexpand(false)
        .css_classes(["card"])
        .build();

    // Si no lo tenemos físico, aplicamos efecto visual
    if view.physical_count == 0 {
        card.add_css_class("missing-comic");
    }

    // Contenedor de la imagen con altura fija, pero ancho flexible
    let image_container = gtk::Box::builder()
        .height_request(ch as i32)
        .halign(gtk::Align::Center)
        .hexpand(false)
        .css_classes(["comic-cover-container"])
        .build();

    // Placeholder inicial: solo define la altura, el ancho será libre
    let img_placeholder = gtk::Image::builder()
        .icon_name("image-x-generic-symbolic")
        .pixel_size((cw / 3) as i32)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .opacity(0.3)
        .build();
    image_container.append(&img_placeholder);

    // Caja de información (Título + número + estado)
    let info_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .margin_top(8)
        .margin_start(4)
        .margin_end(4)
        .margin_bottom(6)
        .halign(gtk::Align::Center)
        .build();

    let title_label = gtk::Label::builder()
        .label(&view.info.titulo)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .lines(2)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .justify(gtk::Justification::Center)
        .halign(gtk::Align::Center)
        .hexpand(false)
        .max_width_chars(12)
        .css_classes(["caption", "heading"])
        .build();
    info_box.append(&title_label);

    let extra_info = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .halign(gtk::Align::Center)
        .build();

    let num = view.info.numero.as_deref().unwrap_or("?");
    extra_info.append(&gtk::Label::builder()
        .label(&format!("#{}", num))
        .css_classes(["dim-label", "caption"])
        .build());

    if view.physical_count > 0 {
        extra_info.append(&gtk::Label::builder()
            .label(&format!("{} físico", view.physical_count))
            .css_classes(["caption", "success"])
            .build());
    } else {
        extra_info.append(&gtk::Label::builder()
            .label("No poseído")
            .css_classes(["caption", "dim-label"])
            .build());
    }

    info_box.append(&extra_info);
    card.append(&image_container);
    card.append(&info_box);

    // Carga asíncrona de la imagen
    let img_weak = image_container.downgrade();
    let volume_name_owned = volume_name.to_string();
    let id_vol = view.info.id_volume.unwrap_or(0);
    let url_original = view.url_original.clone();
    let ruta_cover = view.ruta_cover.clone();

    run_in_background(
        tokio::runtime::Handle::current(),
        async move {
            // 1. Prioridad: Portada descargada (ComicVine)
            let raw = if let Some(path) = ruta_cover {
                tokio::fs::read(&path).await.ok()
            } else if let Some(url) = url_original {
                let filename = url.split('/').last().unwrap_or("");
                if !filename.is_empty() {
                    let thumb = crate::helpers::paths::comicbook_info_thumbnail_path(&volume_name_owned, id_vol, filename);
                    tokio::fs::read(&thumb).await.ok()
                } else {
                    None
                }
            } else {
                None
            };

            // Escalar a la altura del card_size para que el WrapBox calcule bien el ancho
            let raw = raw?;
            tokio::task::spawn_blocking(move || -> Option<Vec<u8>> {
                let img = image::load_from_memory(&raw).ok()?;
                let scaled = img.resize(u32::MAX, ch, image::imageops::FilterType::Triangle);
                let mut out = Vec::new();
                scaled.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Jpeg).ok()?;
                Some(out)
            }).await.ok()?
        },
        move |bytes_opt| {
            if let Some(bytes) = bytes_opt {
                if let Some(container) = img_weak.upgrade() {
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
            }
        }
    );

    card.upcast()
}
