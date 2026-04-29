use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc;

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{self as gtk, gio, glib};
use libadwaita as adw;
use sqlx::SqlitePool;

use babelcomics_core::helpers::download_manager::DownloadManager;
use babelcomics_core::models::{ComicbookInfoView, Volume};
use babelcomics_core::repositories::{ComicbookInfoRepository, PublisherRepository, VolumeRepository};
use crate::ui::run_in_background;

#[derive(Clone, Copy)]
struct ClipDialogStats {
    total_db: i64,
    con_archivo: i64,
    indexadas: i64,
    total_missing_candidates: i64,
    total_all_candidates: i64,
}

fn build_clip_dialog_body(stats: ClipDialogStats) -> String {
    if stats.total_all_candidates == 0 {
        return format!(
            "Este volumen no tiene portadas candidatas para indexar.\nHay {} issues en BD y {} embeddings ya guardados.",
            stats.total_db, stats.indexadas
        );
    }

    if stats.total_missing_candidates == 0 {
        return format!(
            "No quedan faltantes en este volumen.\nYa hay {} portadas indexadas.\nPuedes reindexar {} portadas candidatas si quieres regenerarlas.",
            stats.indexadas, stats.total_all_candidates
        );
    }

    format!(
        "Faltan {} portadas por indexar.\nYa hay {} indexadas de {} con archivo local.\nSi eliges reindexar, se procesarán {} portadas candidatas.",
        stats.total_missing_candidates,
        stats.indexadas,
        stats.con_archivo,
        stats.total_all_candidates
    )
}

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
            VolumeRepository::new(&pool_v)
                .get_by_id(volume_id)
                .await
                .ok()
                .flatten()
        },
        move |vol_opt| {
            let name = vol_opt.map(|v| v.nombre).unwrap_or_default();

            // Pestaña: Información
            let info_content = build_info_tab(volume_id, pool_v2.clone());
            let info_page = tab_view_v.append(&info_content);
            info_page.set_title("Información");
            info_page.set_icon(Some(&gio::ThemedIcon::new("dialog-information-symbolic")));

            // Pestaña: Issues
            let issues_content = build_issues_tab(volume_id, &name, pool_v2, tab_view_v.clone());
            let issues_page = tab_view_v.append(&issues_content);
            issues_page.set_title("Issues");
            issues_page.set_icon(Some(&gio::ThemedIcon::new("view-list-symbolic")));
        },
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
            let vol = VolumeRepository::new(&pool_task)
                .get_by_id(volume_id)
                .await
                .ok()
                .flatten()?;
            let publ = PublisherRepository::new(&pool_task)
                .get_by_id(vol.id_publisher)
                .await
                .ok()
                .flatten();
            let issues = ComicbookInfoRepository::new(&pool_task)
                .get_view_by_volume(volume_id)
                .await
                .unwrap_or_default();

            let owned = issues.iter().filter(|i| i.physical_count > 0).count();
            let total = vol.cantidad_numeros;
            let percent = if total > 0 {
                (owned as f64 / total as f64) * 100.0
            } else {
                0.0
            };

            let first_issue_cover = issues
                .iter()
                .filter_map(|i| i.ruta_cover.as_ref())
                .next()
                .cloned();

            Some((vol, publ, owned, total, percent, first_issue_cover))
        },
        move |res| {
            if let Some((vol, publ, owned, total, percent, first_issue_cover)) = res {
                let content = build_info_content(
                    &vol,
                    publ.as_ref(),
                    owned,
                    total,
                    percent,
                    first_issue_cover,
                    pool.clone(),
                );
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
    publisher: Option<&babelcomics_core::models::Publisher>,
    owned: usize,
    total: i64,
    percent: f64,
    first_issue_cover: Option<String>,
    pool: SqlitePool,
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

    let volume_thumb = babelcomics_core::helpers::paths::volume_thumbnail_path(vol.id_volume);
    if volume_thumb.exists() {
        cover.set_filename(Some(&volume_thumb));
    } else if let Some(path) = first_issue_cover {
        cover.set_filename(Some(std::path::PathBuf::from(path)));
    } else if !vol.image_url.is_empty() {
        // Fallback: descargar desde URL
        let cover_clone = cover.clone();
        let url = vol.image_url.clone();
        let vt_path = volume_thumb.clone();
        run_in_background(
            tokio::runtime::Handle::current(),
            async move {
                if let Some(bytes) = babelcomics_core::helpers::download_manager::fetch_image_bytes(&url).await {
                    if let Some(parent) = vt_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = std::fs::write(&vt_path, &bytes);
                    return Some(vt_path);
                }
                None
            },
            move |path_opt| {
                if let Some(path) = path_opt {
                    cover_clone.set_filename(Some(path));
                }
            },
        );
    }

    header.append(&cover);

    let info_side = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .hexpand(true)
        .build();

    info_side.append(
        &gtk::Label::builder()
            .label(&vol.nombre)
            .halign(gtk::Align::Start)
            .css_classes(["title-1"])
            .wrap(true)
            .build(),
    );

    let stats_group = adw::PreferencesGroup::builder()
        .title("Estadísticas de colección")
        .build();

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

    // --- Herramientas ---
    {
        let tools_group = adw::PreferencesGroup::builder()
            .title("Herramientas")
            .build();

        // ── Actualizar desde ComicVine ──────────────────────────────────────────
        let update_row = adw::ActionRow::builder()
            .title("Actualizar desde ComicVine")
            .subtitle("Descarga issues y portadas usando el ID de ComicVine")
            .build();

        let update_btn = gtk::Button::builder()
            .icon_name("view-refresh-symbolic")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .tooltip_text("Sincronizar volumen con ComicVine")
            .build();

        // Desactivar el botón si el volumen no tiene ID de ComicVine
        let cv_id_opt = vol.id_comicvine;
        if cv_id_opt.is_none() {
            update_btn.set_sensitive(false);
            update_row.set_subtitle("Sin ID de ComicVine asignado");
        }

        {
            let pool_u = pool.clone();
            let vol_nombre = vol.nombre.clone();
            let vol_cantidad = vol.cantidad_numeros;

            update_btn.connect_clicked(move |btn| {
                let cv_id = match cv_id_opt {
                    Some(id) => id,
                    None => return,
                };

                let dialog = adw::AlertDialog::builder()
                    .heading("Actualizar volumen")
                    .body(format!(
                        "Se sincronizará «{}» con ComicVine.\n¿Descargar también las portadas de los issues?",
                        vol_nombre
                    ))
                    .build();
                dialog.add_response("cancel", "Cancelar");
                dialog.add_response("meta", "Solo metadatos");
                dialog.add_response("covers", "Con portadas");
                dialog.set_default_response(Some("meta"));
                dialog.set_close_response("cancel");
                dialog.set_response_appearance("covers", adw::ResponseAppearance::Suggested);

                let pool_d = pool_u.clone();
                let nombre = vol_nombre.clone();
                let parent_widget = btn.root();

                dialog.connect_response(None, move |_d, response| {
                    if response == "cancel" { return; }
                    let download_covers = response == "covers";

                    let dm = DownloadManager::get_instance(pool_d.clone());
                    dm.add_download(cv_id, &nombre, vol_cantidad, download_covers);
                });

                dialog.present(parent_widget.as_ref());
            });
        }

        update_row.add_suffix(&update_btn);
        tools_group.add(&update_row);

        // ── Embeddings CLIP ────────────────────────────────────────────────────
        let clip_row = adw::ActionRow::builder()
            .title("Generar embeddings CLIP")
            .subtitle("Indexa las portadas de este volumen para búsqueda visual")
            .activatable(true)
            .build();

        let clip_btn = gtk::Button::builder()
            .icon_name("media-playback-start-symbolic")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .tooltip_text("Generar embeddings CLIP para este volumen")
            .build();

        let clip_progress_revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .reveal_child(false)
            .build();
        let clip_progress_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(12)
            .margin_end(12)
            .build();
        let clip_progress_title = gtk::Label::builder()
            .label("Generando embeddings CLIP")
            .halign(gtk::Align::Start)
            .css_classes(["heading"])
            .build();
        let clip_progress_subtitle = gtk::Label::builder()
            .label("")
            .halign(gtk::Align::Start)
            .wrap(true)
            .build();
        let clip_progress_bar = gtk::ProgressBar::builder()
            .show_text(true)
            .hexpand(true)
            .build();
        clip_progress_box.append(&clip_progress_title);
        clip_progress_box.append(&clip_progress_subtitle);
        clip_progress_box.append(&clip_progress_bar);
        clip_progress_revealer.set_child(Some(&clip_progress_box));

        let volume_id = vol.id_volume;
        let progress_running = Rc::new(Cell::new(false));
        let clip_progress_revealer_for_click = clip_progress_revealer.clone();
        clip_btn.connect_clicked(move |btn| {
            if progress_running.get() {
                return;
            }

            let pool_d = pool.clone();
            let pool_for_dialog = pool.clone();
            let parent_widget = btn.root();
            let overlay_weak = btn.root()
                .and_then(|r| r.downcast::<adw::ApplicationWindow>().ok())
                .and_then(|w| w.content())
                .and_then(|w| w.downcast::<adw::ToastOverlay>().ok())
                .map(|o| o.downgrade());
            let btn_for_dialog = btn.clone();
            let progress_revealer = clip_progress_revealer_for_click.clone();
            let progress_title = clip_progress_title.clone();
            let progress_subtitle = clip_progress_subtitle.clone();
            let progress_bar = clip_progress_bar.clone();
            let running_flag = progress_running.clone();

            run_in_background(
                tokio::runtime::Handle::current(),
                async move {
                    let repo = ComicbookInfoRepository::new(&pool_d);
                    let stats = repo.count_clip_index_stats(Some(volume_id)).await.ok()?;
                    let total_missing_candidates = repo
                        .get_covers_for_clip(Some(volume_id), true)
                        .await
                        .ok()
                        .map(|covers| covers.len() as i64)
                        .unwrap_or(stats.3);
                    let total_all_candidates = repo
                        .get_covers_for_clip(Some(volume_id), false)
                        .await
                        .ok()
                        .map(|covers| covers.len() as i64)
                        .unwrap_or(stats.0);
                    Some(ClipDialogStats {
                        total_db: stats.0,
                        con_archivo: stats.1,
                        indexadas: stats.2,
                        total_missing_candidates,
                        total_all_candidates,
                    })
                },
                move |stats_opt| {
                    let Some(stats) = stats_opt else {
                        if let Some(overlay) = overlay_weak.as_ref().and_then(|ow| ow.upgrade()) {
                            overlay.add_toast(adw::Toast::builder().title("No se pudo calcular el alcance de CLIP").timeout(4).build());
                        }
                        return;
                    };

                    let dialog = adw::AlertDialog::builder()
                        .heading("Generar embeddings CLIP")
                        .body(build_clip_dialog_body(stats))
                        .build();
                    dialog.add_response("cancel", "Cancelar");
                    dialog.add_response("missing", "Solo faltantes");
                    dialog.add_response("all", "Reindexar todo");
                    dialog.set_default_response(Some("missing"));
                    dialog.set_close_response("cancel");
                    dialog.set_response_appearance("all", adw::ResponseAppearance::Destructive);

                    let pool_start = pool_for_dialog.clone();
                    let overlay_start = overlay_weak.clone();
                    let btn_start = btn_for_dialog.clone();
                    let progress_revealer_start = progress_revealer.clone();
                    let progress_title_start = progress_title.clone();
                    let progress_subtitle_start = progress_subtitle.clone();
                    let progress_bar_start = progress_bar.clone();
                    let running_start = running_flag.clone();

                    dialog.connect_response(None, move |_d, response| {
                        if response == "cancel" {
                            return;
                        }

                        let solo_faltantes = response != "all";
                        let total_target = if solo_faltantes {
                            stats.total_missing_candidates
                        } else {
                            stats.total_all_candidates
                        };

                        running_start.set(true);
                        btn_start.set_sensitive(false);
                        progress_title_start.set_label("Generando embeddings CLIP");
                        progress_subtitle_start.set_label(&format!(
                            "0/{} procesados, {} restantes",
                            total_target,
                            total_target
                        ));
                        progress_bar_start.set_fraction(if total_target > 0 { 0.0 } else { 1.0 });
                        progress_bar_start.set_text(Some(&format!("0/{}", total_target)));
                        progress_revealer_start.set_reveal_child(true);

                        let (event_tx, event_rx) = mpsc::channel::<crate::ui::window::ClipGenerationEvent>();
                        let progress_revealer_poll = progress_revealer_start.clone();
                        let progress_title_poll = progress_title_start.clone();
                        let progress_subtitle_poll = progress_subtitle_start.clone();
                        let progress_bar_poll = progress_bar_start.clone();
                        let btn_poll = btn_start.clone();
                        let running_poll = running_start.clone();

                        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
                            loop {
                                match event_rx.try_recv() {
                                    Ok(crate::ui::window::ClipGenerationEvent::Progress(progress)) => {
                                        let remaining = progress.total.saturating_sub(progress.processed);
                                        let fraction = if progress.total > 0 {
                                            progress.processed as f64 / progress.total as f64
                                        } else {
                                            1.0
                                        };
                                        progress_title_poll.set_label("Generando embeddings CLIP");
                                        progress_subtitle_poll.set_label(&format!(
                                            "{}/{} procesados, {} restantes, {} generados, {} errores",
                                            progress.processed,
                                            progress.total,
                                            remaining,
                                            progress.generated,
                                            progress.errors
                                        ));
                                        progress_bar_poll.set_fraction(fraction);
                                        progress_bar_poll.set_text(Some(&format!(
                                            "{}/{}",
                                            progress.processed,
                                            progress.total
                                        )));
                                    }
                                    Ok(crate::ui::window::ClipGenerationEvent::Finished { result, stats }) => {
                                        btn_poll.set_sensitive(true);
                                        running_poll.set(false);

                                        match result {
                                            Ok((0, _)) if stats.con_archivo == 0 => {
                                                progress_title_poll.set_label("Sin portadas descargadas");
                                                progress_subtitle_poll.set_label(&format!(
                                                    "Hay {} issues en BD, pero ninguna portada local para indexar.",
                                                    stats.total
                                                ));
                                                progress_bar_poll.set_fraction(0.0);
                                                progress_bar_poll.set_text(Some("0/0"));
                                            }
                                            Ok((generated, errs)) => {
                                                let status = if errs.is_empty() {
                                                    "Embeddings CLIP actualizados"
                                                } else {
                                                    "Embeddings CLIP finalizados con errores"
                                                };
                                                progress_title_poll.set_label(status);
                                                progress_subtitle_poll.set_label(&format!(
                                                    "{} generados, {} errores, {} pendientes en el volumen",
                                                    generated,
                                                    errs.len(),
                                                    stats.pendientes.saturating_sub(generated as i64)
                                                ));
                                                progress_bar_poll.set_fraction(1.0);
                                                progress_bar_poll.set_text(Some(&format!("{}", generated)));
                                            }
                                            Err(err) => {
                                                progress_title_poll.set_label("Error generando embeddings CLIP");
                                                progress_subtitle_poll.set_label(&err);
                                                progress_bar_poll.set_fraction(0.0);
                                                progress_bar_poll.set_text(Some("Error"));
                                            }
                                        }

                                        progress_revealer_poll.set_reveal_child(true);
                                        return glib::ControlFlow::Break;
                                    }
                                    Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                                    Err(mpsc::TryRecvError::Disconnected) => {
                                        btn_poll.set_sensitive(true);
                                        running_poll.set(false);
                                        return glib::ControlFlow::Break;
                                    }
                                }
                            }
                        });

                        crate::ui::window::run_clip_generation(
                            Some(volume_id),
                            solo_faltantes,
                            pool_start.clone(),
                            overlay_start.clone(),
                            Some(event_tx),
                        );
                    });

                    dialog.present(parent_widget.as_ref());
                },
            );
        });

        clip_row.add_suffix(&clip_btn);
        tools_group.add(&clip_row);
        tools_group.add(&clip_progress_revealer);
        main_box.append(&tools_group);
    }

    main_box.upcast()
}

// ── Pestaña Issues ─────────────────────────────────────────────────────────────

fn build_issues_tab(
    volume_id: i64,
    volume_name: &str,
    pool: SqlitePool,
    tab_view: adw::TabView,
) -> gtk::Widget {
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
    let offset: Rc<Cell<i64>> = Rc::new(Cell::new(0));
    let loading: Rc<Cell<bool>> = Rc::new(Cell::new(false));
    let all_loaded: Rc<Cell<bool>> = Rc::new(Cell::new(false));
    let query: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let poseidos: Rc<Cell<bool>> = Rc::new(Cell::new(false));
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
            if loading.get() {
                return;
            }
            loading.set(true);

            let pool_t = pool.clone();
            let wb = wrap_box.clone();
            let off = offset.clone();
            let load = loading.clone();
            let done = all_loaded.clone();
            let q = query.borrow().clone();
            let solo = poseidos.get();
            let cur_off = offset.get();
            let v_name_t = v_name.clone();
            let tv_outer = tab_view.clone();
            let pool_outer = pool.clone();

            run_in_background(
                tokio::runtime::Handle::current(),
                async move {
                    let setup = babelcomics_core::repositories::SetupRepository::new(&pool_t)
                        .get()
                        .await
                        .unwrap_or_default();
                    let card_size =
                        babelcomics_core::helpers::thumbnail::CardSize::from_db(setup.thumbnail_size);
                    let issues = ComicbookInfoRepository::new(&pool_t)
                        .get_view_by_volume_page(volume_id, PAGE, cur_off, q.as_deref(), solo)
                        .await
                        .unwrap_or_default();
                    (issues, card_size)
                },
                move |(issues, card_size)| {
                    load.set(false);
                    let received = issues.len() as i64;
                    if received < PAGE {
                        done.set(true);
                    }
                    off.set(cur_off + received);

                    let issues = Rc::new(issues);
                    let idx = Rc::new(Cell::new(0usize));
                    const BATCH: usize = 20;

                    let tab_view_t = tv_outer.clone();
                    let pool_t2 = pool_outer.clone();
                    glib::idle_add_local(move || {
                        let start = idx.get();
                        let end = (start + BATCH).min(issues.len());
                        for issue in &issues[start..end] {
                            wb.append(&build_issue_card(
                                issue,
                                &v_name_t,
                                card_size,
                                tab_view_t.clone(),
                                pool_t2.clone(),
                            ));
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
        let wrap_box = wrap_box.clone();
        let offset = offset.clone();
        let loading = loading.clone();
        let all_loaded = all_loaded.clone();
        let loader = loader.clone();
        move || {
            while let Some(c) = wrap_box.first_child() {
                wrap_box.remove(&c);
            }
            offset.set(0);
            loading.set(false);
            all_loaded.set(false);
            loader.clone()();
        }
    };
    let reset = Rc::new(reset);

    // Búsqueda
    {
        let query = query.clone();
        let se = search_entry.clone();
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
        let reset_p = reset.clone();
        check_poseidos.connect_toggled(move |btn| {
            poseidos.set(btn.is_active());
            reset_p();
        });
    }

    // Scroll infinito
    {
        let adj = scroll.vadjustment();
        let loader_s = loader.clone();
        let loading_s = loading.clone();
        let all_done_s = all_loaded.clone();
        adj.connect_value_changed(move |adj| {
            if loading_s.get() || all_done_s.get() {
                return;
            }
            if adj.value() + adj.page_size() >= adj.upper() - 400.0 {
                loader_s.clone()();
            }
        });
    }

    // Carga inicial
    loader.clone()();

    main_vbox.upcast()
}

fn build_issue_card(
    view: &ComicbookInfoView,
    volume_name: &str,
    card_size: babelcomics_core::helpers::thumbnail::CardSize,
    tab_view: adw::TabView,
    pool: SqlitePool,
) -> gtk::Widget {
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

    // Evento de doble clic para abrir detalle
    let gesture = gtk::GestureClick::new();
    gesture.set_button(1); // Botón izquierdo
    let id_info = view.info.id_comicbook_info;
    let tv = tab_view.clone();
    let pool_click = pool.clone();
    gesture.connect_pressed(move |g, n_press, _, _| {
        if n_press == 2 {
            g.set_state(gtk::EventSequenceState::Claimed);
            let new_page = crate::ui::window::add_tab(
                &tv,
                crate::ui::window::TabKind::IssueDetail(id_info, pool_click.clone()),
            );
            tv.set_selected_page(&new_page);
        }
    });
    card.add_controller(gesture);

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
    extra_info.append(
        &gtk::Label::builder()
            .label(&format!("#{}", num))
            .css_classes(["dim-label", "caption"])
            .build(),
    );

    if view.physical_count > 0 {
        extra_info.append(
            &gtk::Label::builder()
                .label(&format!("{} físico", view.physical_count))
                .css_classes(["caption", "success"])
                .build(),
        );
    } else {
        extra_info.append(
            &gtk::Label::builder()
                .label("No poseído")
                .css_classes(["caption", "dim-label"])
                .build(),
        );
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
            let raw = babelcomics_core::helpers::paths::read_comicbook_info_cover_bytes(
                ruta_cover.as_deref(),
                url_original.as_deref(),
                &volume_name_owned,
                id_vol,
            )
            .await;

            // Escalar a la altura del card_size para que el WrapBox calcule bien el ancho
            let raw = raw?;
            tokio::task::spawn_blocking(move || -> Option<Vec<u8>> {
                babelcomics_core::helpers::thumbnail::resize_to_height_jpeg(&raw, ch)
            })
            .await
            .ok()?
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
        },
    );

    card.upcast()
}
