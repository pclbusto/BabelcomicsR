use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{self as gtk, ScrolledWindow, gdk, glib};
use libadwaita as adw;
use sqlx::SqlitePool;

use babelcomics_core::helpers::paths::comic_thumbnail_path;
use babelcomics_core::helpers::thumbnail::CardSize;
use babelcomics_core::models::VolumeView;
use babelcomics_core::repositories::{SetupRepository, VolumeRepository, VolumeSortOrder};
use crate::ui::run_in_background;

type PublisherFilter = Rc<RefCell<Vec<i64>>>;

/// Registro de IDs de volúmenes (mismo orden que WrapBox children).
type VolumeIdRegistry = Rc<RefCell<Vec<i64>>>;

/// Widgets pendientes de recibir su thumbnail (usamos el id_comicbook_portada).
type ThumbWidgets = Rc<RefCell<HashMap<i64, glib::WeakRef<gtk::Box>>>>;
type ThumbResult = (i64, Vec<u8>);

pub fn build(pool: SqlitePool, tab_view: adw::TabView) -> gtk::Widget {
    let toolbar = adw::ToolbarView::new();

    // Barra de búsqueda
    let search_entry = gtk::SearchEntry::builder()
        .placeholder_text("Buscar series…")
        .hexpand(true)
        .build();

    // --- Popover de ordenación ---
    let sort_popover = gtk::Popover::new();
    let sort_vbox = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_top(10)
        .margin_bottom(10)
        .margin_start(12)
        .margin_end(12)
        .build();

    sort_vbox.append(
        &gtk::Label::builder()
            .label("Ordenar por")
            .halign(gtk::Align::Start)
            .css_classes(["heading"])
            .build(),
    );

    let r_nombre_asc = gtk::CheckButton::with_label("Nombre A–Z");
    let r_nombre_desc = gtk::CheckButton::with_label("Nombre Z–A");
    let r_anio_asc = gtk::CheckButton::with_label("Año ↑");
    let r_anio_desc = gtk::CheckButton::with_label("Año ↓");
    let r_issues_asc = gtk::CheckButton::with_label("Nº issues ↑");
    let r_issues_desc = gtk::CheckButton::with_label("Nº issues ↓");

    r_nombre_asc.set_active(true);
    r_nombre_desc.set_group(Some(&r_nombre_asc));
    r_anio_asc.set_group(Some(&r_nombre_asc));
    r_anio_desc.set_group(Some(&r_nombre_asc));
    r_issues_asc.set_group(Some(&r_nombre_asc));
    r_issues_desc.set_group(Some(&r_nombre_asc));

    for w in [
        &r_nombre_asc,
        &r_nombre_desc,
        &r_anio_asc,
        &r_anio_desc,
        &r_issues_asc,
        &r_issues_desc,
    ] {
        sort_vbox.append(w);
    }
    sort_popover.set_child(Some(&sort_vbox));

    let sort_btn = gtk::MenuButton::builder()
        .icon_name("view-sort-ascending-symbolic")
        .popover(&sort_popover)
        .tooltip_text("Ordenar series")
        .build();

    // --- Popover de filtro por editorial ---
    let filter_popover = gtk::Popover::new();
    let filter_outer = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .margin_top(10)
        .margin_bottom(10)
        .margin_start(12)
        .margin_end(12)
        .build();
    filter_outer.append(
        &gtk::Label::builder()
            .label("Filtrar por editorial")
            .halign(gtk::Align::Start)
            .css_classes(["heading"])
            .build(),
    );
    let filter_scroll = gtk::ScrolledWindow::builder()
        .max_content_height(260)
        .propagate_natural_height(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    let filter_wrap = adw::WrapBox::builder()
        .child_spacing(4)
        .line_spacing(4)
        .build();
    filter_scroll.set_child(Some(&filter_wrap));
    filter_outer.append(&filter_scroll);
    filter_popover.set_child(Some(&filter_outer));

    let filter_btn = gtk::MenuButton::builder()
        .icon_name("building-symbolic")
        .popover(&filter_popover)
        .tooltip_text("Filtrar por editorial")
        .build();

    let search_container = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .halign(gtk::Align::Center)
        .build();
    search_container.append(&search_entry);
    search_container.append(&sort_btn);
    search_container.append(&filter_btn);

    let search_bar = gtk::SearchBar::builder()
        .child(&search_container)
        .show_close_button(true)
        .build();
    search_bar.connect_entry(&search_entry);
    toolbar.add_top_bar(&search_bar);

    let search_bar_capture = search_bar.clone();
    toolbar.connect_realize(move |tb| {
        if let Some(root) = tb.root() {
            search_bar_capture.set_key_capture_widget(Some(&root));
        }
    });

    // Grid de volúmenes
    let scrolled = ScrolledWindow::builder()
        .vexpand(true)
        .hexpand(true)
        .build();

    let wrap_box = adw::WrapBox::builder()
        .valign(gtk::Align::Start)
        .child_spacing(8)
        .line_spacing(8)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(8)
        .margin_end(8)
        .build();

    scrolled.set_child(Some(&wrap_box));

    // ── Registro de IDs de volúmenes ──────────────────────────────────────────
    let volume_id_registry: VolumeIdRegistry = Rc::new(RefCell::new(Vec::new()));

    // ── Menú contextual (click derecho sobre selección) ───────────────────────
    let ctx_popover = gtk::Popover::builder().has_arrow(false).build();

    let popover_vbox = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(0)
        .build();

    // Opción: Actualizar desde ComicVine
    let update_btn = build_popover_button(
        "Sincronizar con ComicVine",
        "Descarga issues y metadatos",
        "view-refresh-symbolic",
    );
    // Opción: Generar Embeddings CLIP
    let clip_btn = build_popover_button(
        "Generar embeddings CLIP",
        "Indexa portadas para búsqueda visual",
        "media-playback-start-symbolic",
    );

    popover_vbox.append(&update_btn);
    popover_vbox.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    popover_vbox.append(&clip_btn);
    ctx_popover.set_child(Some(&popover_vbox));

    {
        let pop = ctx_popover.clone();
        scrolled.connect_realize(move |sw| {
            if pop.parent().is_none() {
                pop.set_parent(sw);
            }
        });
    }

    let placeholder = adw::StatusPage::builder()
        .title("Sin series")
        .description("Las series aparecerán cuando catalogues tus cómics")
        .icon_name("open-book-symbolic")
        .build();

    let spinner = adw::Spinner::builder()
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();

    let stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .build();
    stack.add_named(&spinner, Some("loading"));
    stack.add_named(&placeholder, Some("empty"));
    stack.add_named(&scrolled, Some("grid"));
    stack.set_visible_child_name("loading");

    toolbar.set_content(Some(&stack));

    // --- Sistema de thumbnails ---
    let (thumb_tx, thumb_rx) = mpsc::channel::<ThumbResult>();
    let thumb_widgets: ThumbWidgets = Rc::new(RefCell::new(HashMap::new()));
    start_thumbnail_consumer(thumb_widgets.clone(), thumb_rx, wrap_box.downgrade());

    // --- Estado ---
    let offset: Rc<Cell<i64>> = Rc::new(Cell::new(0));
    let generation: Rc<Cell<u64>> = Rc::new(Cell::new(0));
    let loading: Rc<Cell<bool>> = Rc::new(Cell::new(false));
    let all_loaded: Rc<Cell<bool>> = Rc::new(Cell::new(false));
    let last_clicked: Rc<Cell<i32>> = Rc::new(Cell::new(-1));
    let query = Rc::new(RefCell::new(None::<String>));
    let sort = Rc::new(Cell::new(VolumeSortOrder::default()));
    let selected_publishers: PublisherFilter = Rc::new(RefCell::new(Vec::new()));

    // ── Lógica de acciones por lote ──────────────────────────────────────────
    {
        let wb = wrap_box.clone();
        let registry = volume_id_registry.clone();
        let pool_ctx = pool.clone();
        let popover = ctx_popover.clone();

        // Acción: Actualizar
        let p_upd = pool_ctx.clone();
        let pop_upd = popover.clone();
        let wb_upd = wb.clone();
        let reg_upd = registry.clone();
        update_btn.connect_clicked(move |btn| {
            pop_upd.popdown();
            let selected = get_selected_ids(&wb_upd, &reg_upd);
            if selected.is_empty() {
                return;
            }

            let p_task = p_upd.clone();
            let parent = btn.root();

            let msg = if selected.len() == 1 {
                "¿Actualizar este volumen desde ComicVine?".to_string()
            } else {
                format!(
                    "¿Actualizar {} volúmenes seleccionados desde ComicVine?",
                    selected.len()
                )
            };

            let dialog = adw::AlertDialog::builder()
                .heading("Actualización masiva")
                .body(&msg)
                .build();
            dialog.add_response("cancel", "Cancelar");
            dialog.add_response("meta", "Solo metadatos");
            dialog.add_response("covers", "Con portadas");
            dialog.set_default_response(Some("meta"));
            dialog.set_close_response("cancel");

            dialog.connect_response(None, move |_d, response| {
                if response == "cancel" {
                    return;
                }
                let download_covers = response == "covers";
                let p_loop = p_task.clone();
                let sel_loop = selected.clone();

                run_in_background(
                    tokio::runtime::Handle::current(),
                    async move {
                        let dm = babelcomics_core::helpers::download_manager::DownloadManager::get_instance(
                            p_loop.clone(),
                        );
                        let repo = VolumeRepository::new(&p_loop);
                        for id in sel_loop {
                            if let Ok(Some(vol)) = repo.get_by_id(id).await {
                                if let Some(cv_id) = vol.id_comicvine {
                                    dm.add_download(cv_id, &vol.nombre, vol.cantidad_numeros, download_covers);
                                }
                            }
                        }
                    },
                    |_| {},
                );
            });
            dialog.present(parent.as_ref());
        });

        // Acción: CLIP
        let p_clip = pool_ctx.clone();
        let pop_clip = popover.clone();
        let wb_clip = wb.clone();
        let reg_clip = registry.clone();
        clip_btn.connect_clicked(move |btn| {
            pop_clip.popdown();
            let selected = get_selected_ids(&wb_clip, &reg_clip);
            if selected.is_empty() {
                return;
            }

            let p_task = p_clip.clone();
            let overlay_weak = btn
                .root()
                .and_then(|r| r.downcast::<adw::ApplicationWindow>().ok())
                .and_then(|w| w.content())
                .and_then(|w| w.downcast::<adw::ToastOverlay>().ok())
                .map(|o| o.downgrade());

            let dialog = adw::AlertDialog::builder()
                .heading("Generar embeddings CLIP")
                .body(&format!(
                    "¿Indexar portadas para los {} volúmenes seleccionados?",
                    selected.len()
                ))
                .build();
            dialog.add_response("cancel", "Cancelar");
            dialog.add_response("missing", "Solo faltantes");
            dialog.add_response("all", "Reindexar todo");
            dialog.set_default_response(Some("missing"));
            dialog.set_close_response("cancel");

            dialog.connect_response(None, move |_d, response| {
                if response == "cancel" {
                    return;
                }
                let solo_faltantes = response != "all";
                for id in selected.clone() {
                    crate::ui::window::run_clip_generation(
                        Some(id),
                        solo_faltantes,
                        p_task.clone(),
                        overlay_weak.clone(),
                        None,
                    );
                }
            });
            dialog.present(btn.root().as_ref());
        });
    }

    // Helper de reset + recarga
    let make_reset = {
        let wrap_box = wrap_box.clone();
        let stack = stack.clone();
        let pool = pool.clone();
        let tw = thumb_widgets.clone();
        let tx = thumb_tx.clone();
        let off = offset.clone();
        let load = loading.clone();
        let done = all_loaded.clone();
        let q = query.clone();
        let sort = sort.clone();
        let tv = tab_view.clone();
        let sp = selected_publishers.clone();
        let gen_state = generation.clone();
        let reg = volume_id_registry.clone();
        let lc = last_clicked.clone();
        let pop = ctx_popover.clone();

        move || {
            while let Some(child) = wrap_box.first_child() {
                wrap_box.remove(&child);
            }
            tw.borrow_mut().clear();
            reg.borrow_mut().clear();
            off.set(0);
            load.set(false);
            done.set(false);
            lc.set(-1);

            let new_gen = gen_state.get() + 1;
            gen_state.set(new_gen);

            cargar_pagina(
                &wrap_box,
                &stack,
                &pool,
                &off,
                &load,
                &done,
                &tw,
                &tx,
                q.borrow().clone(),
                sort.get(),
                sp.borrow().clone(),
                true,
                &tv,
                &reg,
                &pop,
                &lc,
                new_gen,
                gen_state.clone(),
            );
        }
    };

    // --- Eventos de búsqueda ---
    {
        let q = query.clone();
        let reset = make_reset.clone();
        search_entry.connect_search_changed(move |se| {
            let text = se.text().to_string();
            *q.borrow_mut() = if text.is_empty() { None } else { Some(text) };
            reset();
        });
    }

    // --- Conexión de los radio buttons de ordenación ---
    {
        let (s, r) = (sort.clone(), make_reset.clone());
        r_nombre_asc.connect_toggled(move |rb: &gtk::CheckButton| {
            if rb.is_active() {
                s.set(VolumeSortOrder::NombreAsc);
                r();
            }
        });
    }
    {
        let (s, r) = (sort.clone(), make_reset.clone());
        r_nombre_desc.connect_toggled(move |rb: &gtk::CheckButton| {
            if rb.is_active() {
                s.set(VolumeSortOrder::NombreDesc);
                r();
            }
        });
    }
    {
        let (s, r) = (sort.clone(), make_reset.clone());
        r_anio_asc.connect_toggled(move |rb: &gtk::CheckButton| {
            if rb.is_active() {
                s.set(VolumeSortOrder::AnioAsc);
                r();
            }
        });
    }
    {
        let (s, r) = (sort.clone(), make_reset.clone());
        r_anio_desc.connect_toggled(move |rb: &gtk::CheckButton| {
            if rb.is_active() {
                s.set(VolumeSortOrder::AnioDesc);
                r();
            }
        });
    }
    {
        let (s, r) = (sort.clone(), make_reset.clone());
        r_issues_asc.connect_toggled(move |rb: &gtk::CheckButton| {
            if rb.is_active() {
                s.set(VolumeSortOrder::IssuesAsc);
                r();
            }
        });
    }
    {
        let (s, r) = (sort.clone(), make_reset.clone());
        r_issues_desc.connect_toggled(move |rb: &gtk::CheckButton| {
            if rb.is_active() {
                s.set(VolumeSortOrder::IssuesDesc);
                r();
            }
        });
    }

    // --- Scroll infinito ---
    {
        let adj = scrolled.vadjustment();
        let (wb_s, stack_s, pool_s) = (wrap_box.clone(), stack.clone(), pool.clone());
        let (off_s, load_s, done_s, lc_s, gen_s) = (
            offset.clone(),
            loading.clone(),
            all_loaded.clone(),
            last_clicked.clone(),
            generation.clone(),
        );
        let tw_s = thumb_widgets.clone();
        let tx_s = thumb_tx.clone();
        let q_s = query.clone();
        let sort_s = sort.clone();
        let tv_s = tab_view.clone();
        let sp_s = selected_publishers.clone();
        let reg_s = volume_id_registry.clone();
        let pop_s = ctx_popover.clone();

        adj.connect_value_changed(move |adj| {
            if load_s.get() || done_s.get() {
                return;
            }
            if adj.value() + adj.page_size() >= adj.upper() - 800.0 {
                let current_gen = gen_s.get();
                cargar_pagina(
                    &wb_s,
                    &stack_s,
                    &pool_s,
                    &off_s,
                    &load_s,
                    &done_s,
                    &tw_s,
                    &tx_s,
                    q_s.borrow().clone(),
                    sort_s.get(),
                    sp_s.borrow().clone(),
                    false,
                    &tv_s,
                    &reg_s,
                    &pop_s,
                    &lc_s,
                    current_gen,
                    gen_s.clone(),
                );
            }
        });
    }

    // --- Cargar editoriales en el popover de filtro ---
    {
        let pool_pub = pool.clone();
        let filter_wrap_clone = filter_wrap.clone();
        let filter_btn_clone = filter_btn.clone();
        let sp = selected_publishers.clone();
        let reset = make_reset.clone();

        run_in_background(
            tokio::runtime::Handle::current(),
            async move {
                VolumeRepository::new(&pool_pub)
                    .get_publishers_in_use()
                    .await
                    .unwrap_or_default()
            },
            move |publishers| {
                for (id, nombre) in publishers {
                    let cb = gtk::CheckButton::with_label(&nombre);
                    let sp2 = sp.clone();
                    let r = reset.clone();
                    let fb = filter_btn_clone.clone();
                    cb.connect_toggled(move |btn| {
                        let mut ids = sp2.borrow_mut();
                        if btn.is_active() {
                            if !ids.contains(&id) {
                                ids.push(id);
                            }
                        } else {
                            ids.retain(|&x| x != id);
                        }
                        let has_filter = !ids.is_empty();
                        drop(ids);
                        if has_filter {
                            fb.add_css_class("accent");
                        } else {
                            fb.remove_css_class("accent");
                        }
                        r();
                    });
                    filter_wrap_clone.append(&cb);
                }
            },
        );
    }

    // Carga inicial
    cargar_pagina(
        &wrap_box,
        &stack,
        &pool,
        &offset,
        &loading,
        &all_loaded,
        &thumb_widgets,
        &thumb_tx,
        None,
        VolumeSortOrder::default(),
        vec![],
        true,
        &tab_view,
        &volume_id_registry,
        &ctx_popover,
        &last_clicked,
        0,
        generation.clone(),
    );

    toolbar.upcast()
}

fn cargar_pagina(
    flow: &adw::WrapBox,
    stack: &gtk::Stack,
    pool: &SqlitePool,
    offset: &Rc<Cell<i64>>,
    loading: &Rc<Cell<bool>>,
    all_loaded: &Rc<Cell<bool>>,
    thumb_widgets: &ThumbWidgets,
    thumb_tx: &mpsc::Sender<ThumbResult>,
    query: Option<String>,
    sort: VolumeSortOrder,
    publisher_ids: Vec<i64>,
    is_first: bool,
    tab_view: &adw::TabView,
    registry: &VolumeIdRegistry,
    ctx_popover: &gtk::Popover,
    last_clicked: &Rc<Cell<i32>>,
    current_gen: u64,
    gen_rc: Rc<Cell<u64>>,
) {
    if loading.get() {
        return;
    }
    loading.set(true);

    if is_first {
        stack.set_visible_child_name("loading");
    }

    let current_offset = offset.get();
    let pool_task = pool.clone();
    let flow_done = flow.clone();
    let stack_done = stack.clone();
    let off_done = offset.clone();
    let load_done = loading.clone();
    let all_done = all_loaded.clone();
    let tw_done = thumb_widgets.clone();
    let tx_done = thumb_tx.clone();
    let tab_view_done = tab_view.clone();
    let registry_done = registry.clone();
    let ctx_popover_done = ctx_popover.clone();
    let lc_done = last_clicked.clone();
    let gen_check = gen_rc;
    let pool_ui = pool.clone();

    run_in_background(
        tokio::runtime::Handle::current(),
        async move {
            let setup = SetupRepository::new(&pool_task)
                .get()
                .await
                .unwrap_or_default();
            let limit = setup.items_por_pagina.max(50);
            let card_size = CardSize::from_db(setup.thumbnail_size);

            let volumes = VolumeRepository::new(&pool_task)
                .get_page(
                    limit,
                    current_offset,
                    query.as_deref(),
                    sort,
                    &publisher_ids,
                )
                .await
                .unwrap_or_default();

            (volumes, limit, card_size)
        },
        move |(volumes, limit, card_size)| {
            load_done.set(false);
            if gen_check.get() != current_gen {
                return;
            }

            let received = volumes.len() as i64;
            if is_first && received == 0 {
                stack_done.set_visible_child_name("empty");
                all_done.set(true);
                return;
            }
            if received < limit {
                all_done.set(true);
            }
            off_done.set(current_offset + received);
            if is_first {
                stack_done.set_visible_child_name("grid");
            }

            let volumes = Rc::new(volumes);
            let idx = Rc::new(RefCell::new(0usize));
            const BATCH: usize = 25;

            glib::idle_add_local(move || {
                if gen_check.get() != current_gen {
                    return glib::ControlFlow::Break;
                }

                let start = *idx.borrow();
                let end = (start + BATCH).min(volumes.len());

                for vol in &volumes[start..end] {
                    let (card, img_weak) = build_volume_card(vol, card_size);
                    adjuntar_gesto_seleccion(
                        &card,
                        &flow_done,
                        &lc_done,
                        vol.id_volume,
                        pool_ui.clone(),
                        tab_view_done.clone(),
                        ctx_popover_done.clone(),
                    );
                    flow_done.append(&card);
                    registry_done.borrow_mut().push(vol.id_volume);

                    if let (Some(id_portada), Some(path)) =
                        (vol.id_comicbook_portada, &vol.path_portada)
                    {
                        tw_done.borrow_mut().insert(id_portada, img_weak);
                        schedule_thumbnail(id_portada, path.clone(), tx_done.clone(), card_size);
                    } else {
                        let vt_path = babelcomics_core::helpers::paths::volume_thumbnail_path(vol.id_volume);
                        if vt_path.exists() {
                            let id_neg = -vol.id_volume;
                            tw_done.borrow_mut().insert(id_neg, img_weak);
                            let tx = tx_done.clone();
                            tokio::runtime::Handle::current().spawn(async move {
                                if let Ok(bytes) = tokio::fs::read(vt_path).await {
                                    let _ = tx.send((id_neg, bytes));
                                }
                            });
                        } else if !vol.image_url.is_empty() {
                            // Fallback final: cargar desde URL si no tenemos nada local
                            let id_neg = -vol.id_volume;
                            tw_done.borrow_mut().insert(id_neg, img_weak);
                            let tx = tx_done.clone();
                            let url = vol.image_url.clone();
                            tokio::runtime::Handle::current().spawn(async move {
                                if let Some(bytes) = babelcomics_core::helpers::download_manager::fetch_image_bytes(&url).await {
                                    if let Some(parent) = vt_path.parent() {
                                        let _ = std::fs::create_dir_all(parent);
                                    }
                                    let _ = std::fs::write(&vt_path, &bytes);
                                    let _ = tx.send((id_neg, bytes));
                                }
                            });
                        }
                    }
                }

                *idx.borrow_mut() = end;
                if end >= volumes.len() {
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            });
        },
    );
}

fn adjuntar_gesto_seleccion(
    card: &gtk::Widget,
    wb: &adw::WrapBox,
    last_clicked: &Rc<Cell<i32>>,
    volume_id: i64,
    pool: SqlitePool,
    tab_view: adw::TabView,
    ctx_popover: gtk::Popover,
) {
    let gesture = gtk::GestureClick::new();
    gesture.set_button(0); // Capturar todos los botones

    let wb_weak = wb.downgrade();
    let card_weak = card.downgrade();
    let lc = last_clicked.clone();
    let tv = tab_view.clone();
    let p = pool.clone();
    let pop = ctx_popover.clone();

    gesture.connect_pressed(move |g, n_press, x, y| {
        let (Some(wb), Some(card)) = (wb_weak.upgrade(), card_weak.upgrade()) else {
            return;
        };

        if n_press >= 2 {
            let page = crate::ui::window::add_tab(
                &tv,
                crate::ui::window::TabKind::VolumeDetail(volume_id, p.clone()),
            );
            tv.set_selected_page(&page);
            g.set_state(gtk::EventSequenceState::Claimed);
            return;
        }

        let button = g.current_button();
        let mods = g.current_event_state();
        let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = mods.contains(gdk::ModifierType::SHIFT_MASK);

        if button == 3 {
            // Click derecho
            let Some(pop_parent) = pop.parent() else {
                return;
            };
            if !card.has_css_class("selected") {
                if !ctrl && !shift {
                    deseleccionar_todo(&wb);
                }
                card.add_css_class("selected");
            }
            let (wx, wy) = card
                .translate_coordinates(&pop_parent, x, y)
                .unwrap_or((x, y));
            pop.set_pointing_to(Some(&gdk::Rectangle::new(wx as i32, wy as i32, 1, 1)));
            pop.popup();
            g.set_state(gtk::EventSequenceState::Claimed);
            return;
        }

        let my_index = wb_child_index(&wb, &card);
        if shift && lc.get() != -1 {
            let start = lc.get().min(my_index as i32);
            let end = lc.get().max(my_index as i32);
            let mut child = wb.first_child();
            let mut i = 0i32;
            while let Some(c) = child {
                if i >= start && i <= end {
                    c.add_css_class("selected");
                } else if !ctrl {
                    c.remove_css_class("selected");
                }
                child = c.next_sibling();
                i += 1;
            }
        } else if ctrl {
            if card.has_css_class("selected") {
                card.remove_css_class("selected");
            } else {
                card.add_css_class("selected");
            }
            lc.set(my_index as i32);
        } else {
            deseleccionar_todo(&wb);
            card.add_css_class("selected");
            lc.set(my_index as i32);
        }
        g.set_state(gtk::EventSequenceState::Claimed);
    });
    card.add_controller(gesture);
}

fn deseleccionar_todo(wb: &adw::WrapBox) {
    let mut child = wb.first_child();
    while let Some(c) = child {
        c.remove_css_class("selected");
        child = c.next_sibling();
    }
}

fn wb_child_index(wb: &adw::WrapBox, target: &gtk::Widget) -> usize {
    let mut child = wb.first_child();
    let mut i = 0usize;
    while let Some(c) = child {
        if &c == target {
            return i;
        }
        child = c.next_sibling();
        i += 1;
    }
    0
}

fn get_selected_ids(wb: &adw::WrapBox, registry: &VolumeIdRegistry) -> Vec<i64> {
    let ids = registry.borrow();
    let mut selected = Vec::new();
    let mut child = wb.first_child();
    let mut i = 0usize;
    while let Some(c) = child {
        if c.has_css_class("selected") {
            if let Some(&id) = ids.get(i) {
                selected.push(id);
            }
        }
        child = c.next_sibling();
        i += 1;
    }
    selected
}

fn build_popover_button(title: &str, subtitle: &str, icon: &str) -> gtk::Button {
    let b = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(8)
        .margin_end(12)
        .build();
    b.append(&gtk::Image::builder().icon_name(icon).pixel_size(16).build());
    let l = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(1)
        .build();
    l.append(
        &gtk::Label::builder()
            .label(title)
            .halign(gtk::Align::Start)
            .build(),
    );
    l.append(
        &gtk::Label::builder()
            .label(subtitle)
            .halign(gtk::Align::Start)
            .css_classes(["caption", "dim-label"])
            .build(),
    );
    b.append(&l);
    gtk::Button::builder()
        .css_classes(["flat"])
        .child(&b)
        .build()
}

fn build_volume_card(
    vol: &VolumeView,
    card_size: CardSize,
) -> (gtk::Widget, glib::WeakRef<gtk::Box>) {
    let (cw, ch) = card_size.dims();

    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .margin_top(4)
        .margin_bottom(4)
        .margin_start(4)
        .margin_end(4)
        .css_classes(["card"])
        .build();

    let image_container = gtk::Box::builder()
        .height_request(ch as i32)
        .overflow(gtk::Overflow::Hidden)
        .css_classes(["comic-cover-container"])
        .build();

    let img = gtk::Image::builder()
        .icon_name("open-book-symbolic")
        .pixel_size((cw / 3) as i32)
        .width_request(cw as i32)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .opacity(0.3)
        .build();
    image_container.append(&img);

    let info_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .margin_top(8)
        .margin_start(4)
        .margin_end(4)
        .margin_bottom(6)
        .build();

    let title = gtk::Label::builder()
        .label(&vol.nombre)
        .wrap(true)
        .lines(2)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .justify(gtk::Justification::Center)
        .max_width_chars(25)
        .css_classes(["caption", "heading"])
        .build();
    info_box.append(&title);

    let sub_info = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .halign(gtk::Align::Center)
        .build();

    if vol.anio_inicio > 0 {
        sub_info.append(
            &gtk::Label::builder()
                .label(&vol.anio_inicio.to_string())
                .css_classes(["caption", "dim-label"])
                .build(),
        );
    }

    let count_text = format!("{} / {}", vol.cantidad_poseida, vol.cantidad_numeros);
    sub_info.append(
        &gtk::Label::builder()
            .label(&count_text)
            .css_classes(["caption", "accent"])
            .build(),
    );

    info_box.append(&sub_info);
    card.append(&image_container);
    card.append(&info_box);

    (card.upcast(), image_container.downgrade())
}

fn start_thumbnail_consumer(
    widgets: ThumbWidgets,
    rx: mpsc::Receiver<ThumbResult>,
    weak_box: glib::WeakRef<adw::WrapBox>,
) {
    let rx = Rc::new(RefCell::new(rx));
    glib::timeout_add_local(Duration::from_millis(16), move || {
        if weak_box.upgrade().is_none() {
            return glib::ControlFlow::Break;
        }
        let mut w = widgets.borrow_mut();
        for _ in 0..32 {
            match rx.borrow().try_recv() {
                Ok((id, bytes)) => {
                    if let Some(weak_box) = w.remove(&id) {
                        if let Some(image_container) = weak_box.upgrade() {
                            let gbytes = glib::Bytes::from_owned(bytes);
                            if let Ok(texture) = gtk::gdk::Texture::from_bytes(&gbytes) {
                                while let Some(child) = image_container.first_child() {
                                    image_container.remove(&child);
                                }
                                let pic = gtk::Picture::for_paintable(&texture);
                                pic.set_content_fit(gtk::ContentFit::Contain);
                                pic.add_css_class("cover-image");
                                image_container.append(&pic);
                            }
                        }
                    }
                }
                _ => break,
            }
        }
        glib::ControlFlow::Continue
    });
}

fn schedule_thumbnail(id: i64, path: String, tx: mpsc::Sender<ThumbResult>, size: CardSize) {
    tokio::runtime::Handle::current().spawn(async move {
        let thumb_path = comic_thumbnail_path(id, size);
        if !thumb_path.exists() {
            let path_clone = path.clone();
            let _ = tokio::task::spawn_blocking(move || {
                if let Ok(bytes) = babelcomics_core::helpers::extractor::extract_cover(&path_clone) {
                    let _ = babelcomics_core::helpers::thumbnail::generate_all_thumbnails(&bytes, id);
                }
            })
            .await;
        }
        for _ in 0..40u8 {
            if let Ok(bytes) = tokio::fs::read(&thumb_path).await {
                let _ = tx.send((id, bytes));
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    });
}
