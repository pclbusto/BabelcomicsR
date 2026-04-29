use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{self as gtk, gio, glib};
use libadwaita as adw;
use sqlx::SqlitePool;

use babelcomics_core::helpers::thumbnail::CardSize;
use babelcomics_core::models::{Comicbook, ComicbookInfo, ComicbookInfoCover};
use babelcomics_core::repositories::{
    ComicbookInfoRepository, ComicbookRepository, SetupRepository, VolumeRepository,
};
use crate::ui::run_in_background;

/// Actualiza el título de la pestaña con el número y título del issue.
pub fn setup_tab_title(issue_info_id: i64, pool: SqlitePool, page: glib::WeakRef<adw::TabPage>) {
    run_in_background(
        tokio::runtime::Handle::current(),
        async move {
            ComicbookInfoRepository::new(&pool)
                .get_by_id(issue_info_id)
                .await
                .ok()
                .flatten()
        },
        move |info_opt| {
            let Some(page) = page.upgrade() else { return };
            let Some(info) = info_opt else { return };
            let display = format!("#{} {}", info.numero.as_deref().unwrap_or("?"), info.titulo);
            page.set_title(&display);
        },
    );
}

pub fn build(issue_info_id: i64, pool: SqlitePool) -> gtk::Widget {
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
    let pool_done = pool.clone();

    run_in_background(
        tokio::runtime::Handle::current(),
        async move {
            let repo = ComicbookInfoRepository::new(&pool_done);
            let info = repo.get_by_id(issue_info_id).await.ok().flatten()?;
            let covers = repo.get_covers(issue_info_id).await.unwrap_or_default();

            let vol_name = if let Some(vid) = info.id_volume {
                VolumeRepository::new(&pool_done)
                    .get_by_id(vid)
                    .await
                    .ok()
                    .flatten()
                    .map(|v| v.nombre)
            } else {
                None
            }
            .unwrap_or_default();

            let setup = SetupRepository::new(&pool_done)
                .get()
                .await
                .unwrap_or_default();
            let card_size = CardSize::from_db(setup.thumbnail_size);

            // También buscamos si tenemos archivos físicos para este issue
            let physical = ComicbookRepository::new(&pool_done)
                .get_by_info_id(issue_info_id)
                .await
                .unwrap_or_default();

            Some((info, covers, physical, card_size, vol_name))
        },
        move |res| {
            if let Some((info, covers, physical, card_size, vol_name)) = res {
                let content =
                    build_content(info, covers, physical, card_size, pool.clone(), &vol_name);
                scroll_done.set_child(Some(&content));
                stack_done.set_visible_child_name("content");
            }
        },
    );

    stack.upcast()
}

fn build_content(
    info: ComicbookInfo,
    covers: Vec<ComicbookInfoCover>,
    physical: Vec<Comicbook>,
    _card_size: CardSize,
    _pool: SqlitePool,
    volume_name: &str,
) -> gtk::Widget {
    let main_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(24)
        .margin_top(24)
        .margin_bottom(24)
        .margin_start(24)
        .margin_end(24)
        .build();

    // --- Parte superior: Carousel de Portadas + Info ---
    let top_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(32)
        .build();

    // Carousel para variantes
    let carousel_container = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .width_request(300)
        .build();

    let carousel = adw::Carousel::builder()
        .width_request(300)
        .height_request(450)
        .spacing(12)
        .build();

    if covers.is_empty() {
        let placeholder = gtk::Image::builder()
            .icon_name("image-x-generic-symbolic")
            .pixel_size(128)
            .opacity(0.3)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .build();
        carousel.append(&placeholder);
    } else {
        for cover in &covers {
            // Contenedor con placeholder mientras carga
            let frame = gtk::Box::builder()
                .width_request(300)
                .height_request(450)
                .halign(gtk::Align::Center)
                .valign(gtk::Align::Center)
                .build();

            frame.append(
                &gtk::Image::builder()
                    .icon_name("image-x-generic-symbolic")
                    .pixel_size(64)
                    .opacity(0.3)
                    .halign(gtk::Align::Center)
                    .valign(gtk::Align::Center)
                    .build(),
            );
            carousel.append(&frame);

            let frame_weak = frame.downgrade();
            let ruta_local = cover.ruta_local.clone();
            let url_original = Some(cover.url_original.clone());
            let vol_nombre = volume_name.to_string();
            let id_vol = info.id_volume.unwrap_or(0);

            run_in_background(
                tokio::runtime::Handle::current(),
                async move {
                    let bytes = babelcomics_core::helpers::paths::read_comicbook_info_cover_bytes(
                        ruta_local.as_deref(),
                        url_original.as_deref(),
                        &vol_nombre,
                        id_vol,
                    )
                    .await?;

                    tokio::task::spawn_blocking(move || {
                        let gbytes = glib::Bytes::from_owned(bytes);
                        gtk::gdk::Texture::from_bytes(&gbytes).ok()
                    })
                    .await
                    .ok()
                    .flatten()
                },
                move |texture_opt| {
                    if let (Some(texture), Some(container)) = (texture_opt, frame_weak.upgrade()) {
                        while let Some(child) = container.first_child() {
                            container.remove(&child);
                        }
                        let pic = gtk::Picture::builder()
                            .paintable(&texture)
                            .content_fit(gtk::ContentFit::Contain)
                            .can_shrink(true)
                            .css_classes(["card"])
                            .build();
                        container.append(&pic);
                    }
                },
            );
        }
    }

    carousel_container.append(&carousel);

    if covers.len() > 1 {
        let dots = adw::CarouselIndicatorDots::builder()
            .carousel(&carousel)
            .halign(gtk::Align::Center)
            .build();
        carousel_container.append(&dots);
    }

    top_box.append(&carousel_container);

    // Información del issue
    let info_side = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(16)
        .hexpand(true)
        .build();

    info_side.append(
        &gtk::Label::builder()
            .label(&format!(
                "#{} {}",
                info.numero.as_deref().unwrap_or("?"),
                info.titulo
            ))
            .halign(gtk::Align::Start)
            .css_classes(["title-1"])
            .wrap(true)
            .build(),
    );

    let details_group = adw::PreferencesGroup::builder()
        .title("Detalles del Issue")
        .build();

    if let Some(cv_id) = info.id_comicvine {
        details_group.add(
            &adw::ActionRow::builder()
                .title("ID ComicVine")
                .subtitle(cv_id.to_string())
                .build(),
        );
    }

    if let Some(rating) = info.calificacion {
        let stars = "★".repeat(rating.round() as usize);
        details_group.add(
            &adw::ActionRow::builder()
                .title("Calificación")
                .subtitle(stars)
                .build(),
        );
    }

    info_side.append(&details_group);

    if let Some(ref resumen) = info.resumen {
        let resumen_group = adw::PreferencesGroup::builder().title("Resumen").build();
        let label = gtk::Label::builder()
            .label(resumen)
            .wrap(true)
            .halign(gtk::Align::Start)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        resumen_group.add(&label);
        info_side.append(&resumen_group);
    }

    top_box.append(&info_side);
    main_box.append(&top_box);

    // --- Archivos Físicos ---
    if !physical.is_empty() {
        let physical_group = adw::PreferencesGroup::builder()
            .title(format!("Archivos en la biblioteca ({})", physical.len()))
            .build();

        for cb in physical {
            let fname = std::path::Path::new(&cb.path)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(&fname).as_str())
                .subtitle(glib::markup_escape_text(&cb.path).as_str())
                .activatable(true)
                .build();

            let btn_read = gtk::Button::builder()
                .icon_name("book-open-symbolic")
                .valign(gtk::Align::Center)
                .css_classes(["flat"])
                .tooltip_text("Leer este archivo")
                .build();

            let path_clone = cb.path.clone();
            btn_read.connect_clicked(move |_| {
                let Some(app) = gio::Application::default() else {
                    return;
                };
                if let Some(adw_app) = app.downcast_ref::<adw::Application>() {
                    crate::ui::reader::ReaderWindow::open(adw_app, &path_clone);
                }
            });
            row.add_suffix(&btn_read);

            physical_group.add(&row);
        }
        main_box.append(&physical_group);
    }

    main_box.upcast()
}
