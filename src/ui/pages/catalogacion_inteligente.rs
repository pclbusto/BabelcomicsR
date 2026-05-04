use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use gtk4::prelude::*;
use gtk4::{self as gtk, glib};
use libadwaita as adw;
use sqlx::SqlitePool;

use crate::ui::run_in_background;
use babelcomics_core::helpers::suggestion_service::UnifiedSuggestion;
use babelcomics_core::helpers::thumbnail::CardSize;
use babelcomics_core::models::ComicbookView;
use babelcomics_core::repositories::ComicbookRepository;

const COVER_HEIGHT: i32 = 400;
const THUMB_SMALL_H: i32 = 80;

pub fn build(comic_ids: Vec<i64>, pool: SqlitePool) -> gtk::Widget {
    // ── Shared state ──────────────────────────────────────────────────────────
    let comic_views: Rc<RefCell<Vec<ComicbookView>>> = Rc::new(RefCell::new(Vec::new()));
    let current_idx: Rc<Cell<usize>> = Rc::new(Cell::new(0));
    let matches: Rc<RefCell<Vec<UnifiedSuggestion>>> = Rc::new(RefCell::new(Vec::new()));
    let selected_match_idx: Rc<Cell<usize>> = Rc::new(Cell::new(0));
    // Incremented each time we switch comic — stale callbacks check this
    let search_gen: Rc<Cell<u64>> = Rc::new(Cell::new(0));

    // Caché de sugerencias pre-calculadas (Thread-safe)
    let suggestions_cache: Arc<Mutex<std::collections::HashMap<i64, Vec<UnifiedSuggestion>>>> =
        Arc::new(Mutex::new(std::collections::HashMap::new()));

    // Caché de Pixbufs ya decodificados y escalados (Thread-safe)
    // Key: (id_comicbook_info, target_height)
    let pixbuf_cache: Arc<Mutex<std::collections::HashMap<(i64, i32), gdk_pixbuf::Pixbuf>>> =
        Arc::new(Mutex::new(std::collections::HashMap::new()));

    // Lista de IDs pendientes de procesar
    let pending_ids: Arc<Mutex<Vec<i64>>> = Arc::new(Mutex::new(comic_ids.clone()));

    // ── Toolbar ───────────────────────────────────────────────────────────────
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::builder()
        .show_start_title_buttons(false)
        .show_end_title_buttons(false)
        .build();
    let title_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .build();
    let title_lbl = gtk::Label::builder()
        .label("Catalogación inteligente")
        .css_classes(["heading"])
        .build();
    let position_lbl = gtk::Label::builder()
        .label("0/0")
        .css_classes(["caption", "dim-label"])
        .build();
    title_box.append(&title_lbl);
    title_box.append(&position_lbl);
    header.set_title_widget(Some(&title_box));

    let apply_btn = gtk::Button::builder()
        .label("Vincular")
        .css_classes(["suggested-action"])
        .sensitive(false)
        .build();
    header.pack_end(&apply_btn);

    let clear_btn = gtk::Button::builder()
        .label("Limpiar vínculo")
        .css_classes(["destructive-action"])
        .tooltip_text("Quita la catalogación del cómic actual")
        .build();
    header.pack_end(&clear_btn);

    let skip_btn = gtk::Button::builder().label("Omitir").build();
    header.pack_end(&skip_btn);
    toolbar.add_top_bar(&header);

    let paned = gtk::Paned::builder()
        .orientation(gtk::Orientation::Horizontal)
        .position(480)
        .hexpand(true)
        .vexpand(true)
        .build();
    toolbar.set_content(Some(&paned));

    // ════════════════════════ LEFT COLUMN ════════════════════════
    let left_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    let left_cover = gtk::Box::builder()
        .height_request(COVER_HEIGHT)
        .halign(gtk::Align::Fill)
        .hexpand(true)
        .overflow(gtk::Overflow::Hidden)
        .css_classes(["comic-cover-container"])
        .build();
    append_placeholder(&left_cover, 64);

    let left_filename = gtk::Label::builder()
        .label("Selecciona un cómic")
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .halign(gtk::Align::Center)
        .margin_top(6)
        .margin_bottom(4)
        .css_classes(["caption", "dim-label"])
        .build();

    let left_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .css_classes(["navigation-sidebar"])
        .build();
    let left_scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .child(&left_list)
        .build();

    left_box.append(&left_cover);
    left_box.append(&left_filename);
    left_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    left_box.append(&left_scroll);
    paned.set_start_child(Some(&left_box));

    // ════════════════════════ RIGHT COLUMN ════════════════════════
    let right_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    let right_cover = gtk::Box::builder()
        .height_request(COVER_HEIGHT)
        .halign(gtk::Align::Fill)
        .hexpand(true)
        .overflow(gtk::Overflow::Hidden)
        .css_classes(["comic-cover-container"])
        .build();
    let right_info = gtk::Label::builder()
        .label("Esperando selección…")
        .halign(gtk::Align::Center)
        .margin_top(6)
        .css_classes(["caption"])
        .build();
    let candidates_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_top(8)
        .build();
    let candidates_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .hexpand(true)
        .min_content_height(THUMB_SMALL_H + 20)
        .child(&candidates_box)
        .build();

    right_box.append(&right_cover);
    right_box.append(&right_info);
    right_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    right_box.append(&candidates_scroll);
    paned.set_end_child(Some(&right_box));

    // ── Row selection logic ──────────────────────────────────────────────────
    {
        let comic_views = comic_views.clone();
        let current_idx = current_idx.clone();
        let matches = matches.clone();
        let selected_match_idx = selected_match_idx.clone();
        let search_gen = search_gen.clone();
        let left_cover = left_cover.clone();
        let left_filename = left_filename.clone();
        let right_cover = right_cover.clone();
        let right_info = right_info.clone();
        let candidates_box = candidates_box.clone();
        let apply_btn = apply_btn.clone();
        let position_lbl = position_lbl.clone();
        let pool = pool.clone();
        let cache = suggestions_cache.clone();
        let p_cache = pixbuf_cache.clone();

        left_list.connect_row_selected(move |_, row_opt| {
            let Some(row) = row_opt else { return };
            let idx = row.index() as usize;
            current_idx.set(idx);
            selected_match_idx.set(0);
            matches.borrow_mut().clear();
            apply_btn.set_sensitive(false);
            let total = comic_views.borrow().len();
            position_lbl.set_label(&format!("{}/{}", idx + 1, total));

            let comic_id = {
                let views = comic_views.borrow();
                let Some(comic) = views.get(idx) else { return };
                let filename = std::path::Path::new(&comic.path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| comic.path.clone());
                left_filename.set_label(&filename);
                comic.id_comicbook
            };

            clear_box(&left_cover);
            append_placeholder(&left_cover, 64);
            load_comic_cover_async(comic_id, left_cover.clone());

            let generation = search_gen.get() + 1;
            search_gen.set(generation);

            // Consultar Caché
            let cached = { cache.lock().unwrap().get(&comic_id).cloned() };
            if let Some(c_matches) = cached {
                *matches.borrow_mut() = c_matches.clone();
                populate_right_column(
                    &c_matches,
                    0,
                    &right_cover,
                    &right_info,
                    &candidates_box,
                    &apply_btn,
                    matches.clone(),
                    selected_match_idx.clone(),
                    p_cache.clone(),
                );
                return;
            }

            clear_box(&right_cover);
            right_cover.append(
                &adw::Spinner::builder()
                    .halign(gtk::Align::Center)
                    .valign(gtk::Align::Center)
                    .build(),
            );
            right_info.set_label("Buscando coincidencias…");
            clear_box(&candidates_box);

            let p = pool.clone();
            let m_ui = matches.clone();
            let smi = selected_match_idx.clone();
            let s_gen = search_gen.clone();
            let r_cov = right_cover.clone();
            let r_inf = right_info.clone();
            let c_box = candidates_box.clone();
            let a_btn = apply_btn.clone();
            let cache_cb = cache.clone();
            let pc_cb = p_cache.clone();

            run_in_background(
                tokio::runtime::Handle::current(),
                async move {
                    babelcomics_core::helpers::suggestion_service::suggest_best_matches(
                        &p, comic_id, 10,
                    )
                    .await
                },
                move |result| {
                    if s_gen.get() != generation {
                        return;
                    }
                    match result {
                        Ok(new_matches) => {
                            cache_cb
                                .lock()
                                .unwrap()
                                .insert(comic_id, new_matches.clone());
                            *m_ui.borrow_mut() = new_matches.clone();
                            populate_right_column(
                                &new_matches,
                                0,
                                &r_cov,
                                &r_inf,
                                &c_box,
                                &a_btn,
                                m_ui,
                                smi,
                                pc_cb,
                            );
                        }
                        Err(e) => {
                            clear_box(&r_cov);
                            append_placeholder(&r_cov, 64);
                            r_inf.set_label(&format!("Error: {e}"));
                        }
                    }
                },
            );
        });
    }

    // ── Clear cataloguing logic ──────────────────────────────────────────────
    {
        let comic_views = comic_views.clone();
        let current_idx = current_idx.clone();
        let left_list = left_list.clone();
        let pool = pool.clone();

        clear_btn.connect_clicked(move |btn| {
            let idx = current_idx.get();
            let comic_id = comic_views.borrow().get(idx).map(|c| c.id_comicbook);
            let Some(cid) = comic_id else { return };

            btn.set_sensitive(false);
            let p = pool.clone();
            let btn_done = btn.clone();
            let views_done = comic_views.clone();
            let ll_done = left_list.clone();

            run_in_background(
                tokio::runtime::Handle::current(),
                async move {
                    babelcomics_core::repositories::ComicbookRepository::new(&p)
                        .set_info(cid, None)
                        .await
                },
                move |_| {
                    if let Some(view) = views_done.borrow_mut().get_mut(idx) {
                        view.titulo = None;
                        view.numero = None;
                        view.calificacion = None;
                        view.nombre_volume = None;
                        view.nombre_publisher = None;
                        view.ruta_cover = None;
                        view.catalog_match_similarity = None;
                        view.catalog_best_similarity = None;
                        view.catalog_selected_rank = None;
                        view.catalog_match_method = None;

                        if let Some(row) = ll_done.row_at_index(idx as i32) {
                            set_list_row_title(&row, &display_title_for_view(view));
                        }
                    }
                    btn_done.set_sensitive(true);
                },
            );
        });
    }

    // ── Background Prefetcher ────────────────────────────────────────────────
    {
        let pool_bg = pool.clone();
        let cache_bg = suggestions_cache.clone();
        let pending = pending_ids.clone();
        tokio::runtime::Handle::current().spawn(async move {
            loop {
                let next_id = {
                    let mut p = pending.lock().unwrap();
                    if p.is_empty() {
                        break;
                    }
                    p.remove(0)
                };
                if cache_bg.lock().unwrap().contains_key(&next_id) {
                    continue;
                }
                if let Ok(res) =
                    babelcomics_core::helpers::suggestion_service::suggest_best_matches(
                        &pool_bg, next_id, 10,
                    )
                    .await
                {
                    cache_bg.lock().unwrap().insert(next_id, res);
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        });
    }

    // ── Skip logic ──────────────────────────────────────────────────────────
    {
        let comic_views = comic_views.clone();
        let current_idx = current_idx.clone();
        let left_list = left_list.clone();

        skip_btn.connect_clicked(move |_| {
            let idx = current_idx.get();
            let total = comic_views.borrow().len();
            if idx + 1 < total {
                if let Some(row) = left_list.row_at_index((idx + 1) as i32) {
                    left_list.select_row(Some(&row));
                }
            }
        });
    }

    // ── Apply logic ──────────────────────────────────────────────────────────
    {
        let comic_views = comic_views.clone();
        let current_idx = current_idx.clone();
        let matches = matches.clone();
        let selected_match_idx = selected_match_idx.clone();
        let left_list = left_list.clone();
        let pool = pool.clone();

        apply_btn.connect_clicked(move |btn| {
            let idx = current_idx.get();
            let comic_id = comic_views.borrow().get(idx).map(|c| c.id_comicbook);
            let info_id = matches
                .borrow()
                .get(selected_match_idx.get())
                .map(|m| m.id_comicbook_info);
            let match_metrics = {
                let matches_ref = matches.borrow();
                let selected_idx = selected_match_idx.get();
                matches_ref.get(selected_idx).map(|selected| {
                    let best_similarity = matches_ref
                        .first()
                        .map(|m| m.similarity)
                        .unwrap_or(selected.similarity);
                    let method = match selected.method {
                        babelcomics_core::helpers::suggestion_service::SuggestionMethod::Clip => {
                            "clip"
                        }
                        babelcomics_core::helpers::suggestion_service::SuggestionMethod::Hash => {
                            "hash"
                        }
                    };
                    (
                        selected.similarity as f64,
                        best_similarity as f64,
                        selected_idx as i64 + 1,
                        method.to_string(),
                    )
                })
            };

            if let (
                Some(cid),
                Some(iid),
                Some((selected_similarity, best_similarity, rank, method)),
            ) = (comic_id, info_id, match_metrics)
            {
                btn.set_sensitive(false);
                let p = pool.clone();
                let ll = left_list.clone();
                let total = comic_views.borrow().len();
                run_in_background(
                    tokio::runtime::Handle::current(),
                    async move {
                        babelcomics_core::repositories::ComicbookRepository::new(&p)
                            .set_info_with_match_metrics(
                                cid,
                                iid,
                                selected_similarity,
                                best_similarity,
                                rank,
                                &method,
                            )
                            .await
                    },
                    move |_| {
                        if idx + 1 < total {
                            if let Some(row) = ll.row_at_index((idx + 1) as i32) {
                                ll.select_row(Some(&row));
                            }
                        }
                    },
                );
            }
        });
    }

    // ── Initial load ──────────────────────────────────────────────────────────
    {
        let p_load = pool.clone();
        let cv_init = comic_views.clone();
        let ll_init = left_list.clone();
        run_in_background(
            tokio::runtime::Handle::current(),
            async move {
                let repo = ComicbookRepository::new(&p_load);
                let mut views = Vec::new();
                for id in &comic_ids {
                    if let Ok(Some(v)) = repo.get_view_by_id(*id).await {
                        views.push(v);
                    }
                }
                views
            },
            move |views| {
                {
                    *cv_init.borrow_mut() = views.clone();
                }
                for v in &views {
                    ll_init.append(&build_list_row(v));
                }
                if let Some(first) = ll_init.row_at_index(0) {
                    ll_init.select_row(Some(&first));
                }
            },
        );
    }

    toolbar.upcast()
}

fn clear_box(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

fn append_placeholder(b: &gtk::Box, px: i32) {
    let img = gtk::Image::builder()
        .icon_name("image-x-generic-symbolic")
        .pixel_size(px)
        .opacity(0.3)
        .build();
    b.append(&img);
}

fn load_comic_cover_async(comic_id: i64, target: gtk::Box) {
    let thumb_path =
        babelcomics_core::helpers::paths::comic_thumbnail_path(comic_id, CardSize::Large);
    run_in_background(
        tokio::runtime::Handle::current(),
        async move { tokio::fs::read(&thumb_path).await.ok() },
        move |bytes_opt| {
            if let (Some(bytes), Some(target)) = (bytes_opt, Some(target)) {
                let gbytes = glib::Bytes::from_owned(bytes);
                if let Ok(texture) = gtk::gdk::Texture::from_bytes(&gbytes) {
                    clear_box(&target);
                    let pic = gtk::Picture::for_paintable(&texture);
                    pic.set_content_fit(gtk::ContentFit::Contain);
                    target.append(&pic);
                }
            }
        },
    );
}

fn build_list_row(view: &ComicbookView) -> gtk::ListBoxRow {
    let row_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .margin_top(4)
        .margin_bottom(4)
        .margin_start(6)
        .margin_end(6)
        .build();
    let thumb_container = gtk::Box::builder()
        .width_request(48)
        .height_request(THUMB_SMALL_H)
        .overflow(gtk::Overflow::Hidden)
        .css_classes(["comic-cover-container"])
        .build();
    append_placeholder(&thumb_container, 24);

    let id = view.id_comicbook;
    let tc_weak = thumb_container.downgrade();
    glib::idle_add_local_once(move || {
        let thumb_path =
            babelcomics_core::helpers::paths::comic_thumbnail_path(id, CardSize::Small);
        run_in_background(
            tokio::runtime::Handle::current(),
            async move { tokio::fs::read(&thumb_path).await.ok() },
            move |bytes_opt| {
                if let (Some(bytes), Some(target)) = (bytes_opt, tc_weak.upgrade()) {
                    let gbytes = glib::Bytes::from_owned(bytes);
                    if let Ok(texture) = gtk::gdk::Texture::from_bytes(&gbytes) {
                        clear_box(&target);
                        let pic = gtk::Picture::for_paintable(&texture);
                        pic.set_content_fit(gtk::ContentFit::Contain);
                        target.append(&pic);
                    }
                }
            },
        );
    });

    let label_text = display_title_for_view(view);
    row_box.append(&thumb_container);
    row_box.append(
        &gtk::Label::builder()
            .label(&label_text)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .halign(gtk::Align::Start)
            .hexpand(true)
            .max_width_chars(24)
            .build(),
    );
    gtk::ListBoxRow::builder().child(&row_box).build()
}

fn display_title_for_view(view: &ComicbookView) -> String {
    if let Some(t) = &view.titulo {
        t.clone()
    } else {
        std::path::Path::new(&view.path)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| view.path.clone())
    }
}

fn set_list_row_title(row: &gtk::ListBoxRow, title: &str) {
    let Some(row_box) = row.child().and_then(|w| w.downcast::<gtk::Box>().ok()) else {
        return;
    };
    let mut child = row_box.first_child();
    while let Some(widget) = child {
        if let Ok(label) = widget.clone().downcast::<gtk::Label>() {
            label.set_label(title);
            return;
        }
        child = widget.next_sibling();
    }
}

#[allow(clippy::too_many_arguments)]
fn populate_right_column(
    new_matches: &[UnifiedSuggestion],
    active_idx: usize,
    right_cover: &gtk::Box,
    right_info: &gtk::Label,
    candidates_box: &gtk::Box,
    apply_btn: &gtk::Button,
    matches: Rc<RefCell<Vec<UnifiedSuggestion>>>,
    smi: Rc<Cell<usize>>,
    pc: Arc<Mutex<std::collections::HashMap<(i64, i32), gdk_pixbuf::Pixbuf>>>,
) {
    if new_matches.is_empty() {
        clear_box(right_cover);
        append_placeholder(right_cover, 64);
        right_info.set_label("Sin coincidencias encontradas");
        apply_btn.set_sensitive(false);
        return;
    }
    show_match_in_cover(new_matches, active_idx, right_cover, right_info, pc.clone());
    apply_btn.set_sensitive(true);
    clear_box(candidates_box);
    for (i, m) in new_matches.iter().enumerate() {
        let thumb = build_candidate_thumb(m, i == active_idx, pc.clone());
        let m_cap = matches.clone();
        let smi_cap = smi.clone();
        let r_cov = right_cover.clone();
        let r_inf = right_info.clone();
        let c_box = candidates_box.clone();
        let a_btn = apply_btn.clone();
        let pc_cap = pc.clone();
        let idx = i;
        let gesture = gtk::GestureClick::new();
        gesture.connect_pressed(move |g, _, _, _| {
            smi_cap.set(idx);
            let m_vec = m_cap.borrow();
            show_match_in_cover(&m_vec, idx, &r_cov, &r_inf, pc_cap.clone());
            let mut child = c_box.first_child();
            let mut j = 0usize;
            while let Some(c) = child {
                if j == idx {
                    c.add_css_class("selected");
                } else {
                    c.remove_css_class("selected");
                }
                child = c.next_sibling();
                j += 1;
            }
            a_btn.set_sensitive(true);
            g.set_state(gtk::EventSequenceState::Claimed);
        });
        thumb.add_controller(gesture);
        candidates_box.append(&thumb);
    }
}

fn show_match_in_cover(
    matches: &[UnifiedSuggestion],
    idx: usize,
    right_cover: &gtk::Box,
    right_info: &gtk::Label,
    pc: Arc<Mutex<std::collections::HashMap<(i64, i32), gdk_pixbuf::Pixbuf>>>,
) {
    let Some(m) = matches.get(idx) else { return };
    let method = match m.method {
        babelcomics_core::helpers::suggestion_service::SuggestionMethod::Clip => "CLIP · ",
        babelcomics_core::helpers::suggestion_service::SuggestionMethod::Hash => "Hash · ",
    };
    right_info.set_label(&format!(
        "{}{} #{} ({:.0}%)",
        method,
        m.titulo,
        m.numero.as_deref().unwrap_or("?"),
        m.similarity * 100.0
    ));

    let info_id = m.id_comicbook_info;
    let target_h = COVER_HEIGHT;

    // Check cache
    if let Some(pixbuf) = pc.lock().unwrap().get(&(info_id, target_h)).cloned() {
        clear_box(right_cover);
        let pic = gtk::Picture::for_pixbuf(&pixbuf);
        pic.set_content_fit(gtk::ContentFit::Contain);
        right_cover.append(&pic);
        return;
    }

    clear_box(right_cover);
    append_placeholder(right_cover, 64);
    let cover_weak = right_cover.downgrade();
    let r_c = m.ruta_cover.clone();
    let u_o = m.url_original.clone();
    let n_v = m.nombre_volume.clone().unwrap_or_default();
    let i_v = m.id_volume.unwrap_or(0);
    let pc_clone = pc.clone();
    run_in_background(
        tokio::runtime::Handle::current(),
        async move {
            let bytes = babelcomics_core::helpers::paths::read_comicbook_info_cover_bytes(
                r_c.as_deref(),
                u_o.as_deref(),
                &n_v,
                i_v,
            )
            .await?;
            tokio::task::spawn_blocking(move || {
                babelcomics_core::helpers::thumbnail::resize_to_height_rgb(&bytes, target_h as u32)
            })
            .await
            .ok()?
        },
        move |pixels_opt| {
            if let (Some((data, w, h, rs)), Some(container)) = (pixels_opt, cover_weak.upgrade()) {
                let pixbuf = gdk_pixbuf::Pixbuf::from_bytes(
                    &glib::Bytes::from_owned(data),
                    gdk_pixbuf::Colorspace::Rgb,
                    false,
                    8,
                    w,
                    h,
                    rs,
                );
                pc_clone
                    .lock()
                    .unwrap()
                    .insert((info_id, target_h), pixbuf.clone());
                clear_box(&container);
                let pic = gtk::Picture::for_pixbuf(&pixbuf);
                pic.set_content_fit(gtk::ContentFit::Contain);
                container.append(&pic);
            }
        },
    );
}

fn build_candidate_thumb(
    m: &UnifiedSuggestion,
    is_active: bool,
    pc: Arc<Mutex<std::collections::HashMap<(i64, i32), gdk_pixbuf::Pixbuf>>>,
) -> gtk::Box {
    let outer = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .build();
    if is_active {
        outer.add_css_class("selected");
    }
    outer.add_css_class("card");
    let cover_box = gtk::Box::builder()
        .width_request(60)
        .height_request(THUMB_SMALL_H)
        .overflow(gtk::Overflow::Hidden)
        .css_classes(["comic-cover-container"])
        .build();

    let info_id = m.id_comicbook_info;
    let target_h = THUMB_SMALL_H;

    // Check cache
    if let Some(pixbuf) = pc.lock().unwrap().get(&(info_id, target_h)).cloned() {
        let pic = gtk::Picture::for_pixbuf(&pixbuf);
        pic.set_content_fit(gtk::ContentFit::Contain);
        cover_box.append(&pic);
    } else {
        append_placeholder(&cover_box, 24);
        let cover_weak = cover_box.downgrade();
        let r_c = m.ruta_cover.clone();
        let u_o = m.url_original.clone();
        let n_v = m.nombre_volume.clone().unwrap_or_default();
        let i_v = m.id_volume.unwrap_or(0);
        let pc_clone = pc.clone();
        run_in_background(
            tokio::runtime::Handle::current(),
            async move {
                let bytes = babelcomics_core::helpers::paths::read_comicbook_info_cover_bytes(
                    r_c.as_deref(),
                    u_o.as_deref(),
                    &n_v,
                    i_v,
                )
                .await?;
                tokio::task::spawn_blocking(move || {
                    babelcomics_core::helpers::thumbnail::resize_to_height_rgb(
                        &bytes,
                        target_h as u32,
                    )
                })
                .await
                .ok()?
            },
            move |pixels_opt| {
                if let (Some((data, w, h, rs)), Some(container)) =
                    (pixels_opt, cover_weak.upgrade())
                {
                    let pixbuf = gdk_pixbuf::Pixbuf::from_bytes(
                        &glib::Bytes::from_owned(data),
                        gdk_pixbuf::Colorspace::Rgb,
                        false,
                        8,
                        w,
                        h,
                        rs,
                    );
                    pc_clone
                        .lock()
                        .unwrap()
                        .insert((info_id, target_h), pixbuf.clone());
                    clear_box(&container);
                    let pic = gtk::Picture::for_pixbuf(&pixbuf);
                    pic.set_content_fit(gtk::ContentFit::Contain);
                    container.append(&pic);
                }
            },
        );
    }

    outer.append(&cover_box);
    outer.append(
        &gtk::Label::builder()
            .label(&format!("{:.0}%", m.similarity * 100.0))
            .css_classes(["caption"])
            .halign(gtk::Align::Center)
            .build(),
    );
    outer
}
