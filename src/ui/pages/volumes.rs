use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use gtk4::prelude::*;
use gtk4::{self as gtk, glib, ScrolledWindow, gdk};
use libadwaita as adw;
use sqlx::SqlitePool;

use crate::helpers::paths::comic_thumbnail_path;
use crate::helpers::thumbnail::CardSize;
use crate::models::VolumeView;
use crate::repositories::{VolumeRepository, SetupRepository};
use crate::ui::run_in_background;

/// Widgets pendientes de recibir su thumbnail (usamos el id_comicbook_portada).
type ThumbWidgets = Rc<RefCell<HashMap<i64, glib::WeakRef<gtk::Box>>>>;
type ThumbResult = (i64, Vec<u8>);

pub fn build(pool: SqlitePool, tab_view: adw::TabView) -> gtk::Widget {
    let toolbar = adw::ToolbarView::new();

    // Barra de búsqueda: oculta, aparece al tipear (igual que en comics.rs)
    let search_entry = gtk::SearchEntry::builder()
        .placeholder_text("Buscar series…")
        .hexpand(true)
        .build();

    let search_bar = gtk::SearchBar::builder()
        .child(&search_entry)
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
    let offset = Rc::new(Cell::new(0i64));
    let loading = Rc::new(Cell::new(false));
    let all_loaded = Rc::new(Cell::new(false));
    let query = Rc::new(RefCell::new(None::<String>));

    // --- Eventos de búsqueda ---
    {
        let pool = pool.clone();
        let wrap_box = wrap_box.clone();
        let stack = stack.clone();
        let tw = thumb_widgets.clone();
        let tx = thumb_tx.clone();
        let off = offset.clone();
        let load = loading.clone();
        let done = all_loaded.clone();
        let q = query.clone();
        let tv = tab_view.clone();

        search_entry.connect_search_changed(move |se| {
            let text = se.text().to_string();
            let new_q = if text.is_empty() { None } else { Some(text) };
            *q.borrow_mut() = new_q;

            while let Some(child) = wrap_box.first_child() { wrap_box.remove(&child); }
            tw.borrow_mut().clear();
            off.set(0);
            load.set(false);
            done.set(false);

            cargar_pagina(&wrap_box, &stack, &pool, &off, &load, &done, &tw, &tx, q.borrow().clone(), true, &tv);
        });
    }

    // --- Scroll infinito ---
    {
        let adj = scrolled.vadjustment();
        let (wb_s, stack_s, pool_s) = (wrap_box.clone(), stack.clone(), pool.clone());
        let (off_s, load_s, done_s) = (offset.clone(), loading.clone(), all_loaded.clone());
        let tw_s = thumb_widgets.clone();
        let tx_s = thumb_tx.clone();
        let q_s = query.clone();
        let tv_s = tab_view.clone();

        adj.connect_value_changed(move |adj| {
            if load_s.get() || done_s.get() { return; }
            if adj.value() + adj.page_size() >= adj.upper() - 800.0 {
                cargar_pagina(&wb_s, &stack_s, &pool_s, &off_s, &load_s, &done_s, &tw_s, &tx_s, q_s.borrow().clone(), false, &tv_s);
            }
        });
    }

    // Carga inicial
    cargar_pagina(&wrap_box, &stack, &pool, &offset, &loading, &all_loaded, &thumb_widgets, &thumb_tx, None, true, &tab_view);

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
    is_first: bool,
    tab_view: &adw::TabView,
) {
    if loading.get() { return; }
    loading.set(true);

    if is_first {
        stack.set_visible_child_name("loading");
    }

    let current_offset = offset.get();
    let pool_task = pool.clone();
    let pool_done = pool.clone();
    let flow_done = flow.clone();
    let stack_done = stack.clone();
    let off_done = offset.clone();
    let load_done = loading.clone();
    let all_done = all_loaded.clone();
    let tw_done = thumb_widgets.clone();
    let tx_done = thumb_tx.clone();
    let tab_view_done = tab_view.clone();

    run_in_background(
        tokio::runtime::Handle::current(),
        async move {
            let setup = SetupRepository::new(&pool_task).get().await.unwrap_or_default();
            let limit = setup.items_por_pagina.max(50);
            let card_size = CardSize::from_db(setup.thumbnail_size);

            let volumes = VolumeRepository::new(&pool_task)
                .get_page(limit, current_offset, query.as_deref())
                .await
                .unwrap_or_default();

            (volumes, limit, card_size)
        },
        move |(volumes, limit, card_size)| {
            load_done.set(false);
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
                let start = *idx.borrow();
                let end = (start + BATCH).min(volumes.len());

                for vol in &volumes[start..end] {
                    let (card, img_weak) = build_volume_card(vol, card_size);
                    adjuntar_gesto_clic(&card, vol.id_volume, pool_done.clone(), tab_view_done.clone());
                    flow_done.append(&card);

                    if let (Some(id_portada), Some(path)) = (vol.id_comicbook_portada, &vol.path_portada) {
                        tw_done.borrow_mut().insert(id_portada, img_weak);
                        schedule_thumbnail(id_portada, path.clone(), tx_done.clone(), card_size);
                    } else {
                        let vt_path = crate::helpers::paths::volume_thumbnail_path(vol.id_volume);
                        if vt_path.exists() {
                            let id_neg = -vol.id_volume;
                            tw_done.borrow_mut().insert(id_neg, img_weak);
                            let tx = tx_done.clone();
                            tokio::runtime::Handle::current().spawn(async move {
                                if let Ok(bytes) = tokio::fs::read(vt_path).await {
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

fn adjuntar_gesto_clic(card: &gtk::Widget, volume_id: i64, pool: SqlitePool, tab_view: adw::TabView) {
    let gesture = gtk::GestureClick::new();
    gesture.connect_pressed(move |g, n_press, _, _| {
        if n_press >= 2 {
            let mods = g.current_event_state();
            let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);

            let page = crate::ui::window::add_tab(
                &tab_view,
                crate::ui::window::TabKind::VolumeDetail(volume_id, pool.clone()),
            );
            
            if !ctrl {
                tab_view.set_selected_page(&page);
            }
            g.set_state(gtk::EventSequenceState::Claimed);
        }
    });
    card.add_controller(gesture);
}

fn build_volume_card(vol: &VolumeView, card_size: CardSize) -> (gtk::Widget, glib::WeakRef<gtk::Box>) {
    let (cw, ch) = card_size.dims();
    
    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .margin_top(4).margin_bottom(4).margin_start(4).margin_end(4)
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
        .margin_top(8).margin_start(4).margin_end(4).margin_bottom(6)
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
        sub_info.append(&gtk::Label::builder()
            .label(&vol.anio_inicio.to_string())
            .css_classes(["caption", "dim-label"])
            .build());
    }

    let count_text = format!("{} / {}", vol.cantidad_poseida, vol.cantidad_numeros);
    sub_info.append(&gtk::Label::builder()
        .label(&count_text)
        .css_classes(["caption", "accent"])
        .build());

    info_box.append(&sub_info);
    card.append(&image_container);
    card.append(&info_box);

    (card.upcast(), image_container.downgrade())
}

fn start_thumbnail_consumer(widgets: ThumbWidgets, rx: mpsc::Receiver<ThumbResult>, weak_box: glib::WeakRef<adw::WrapBox>) {
    let rx = Rc::new(RefCell::new(rx));
    glib::timeout_add_local(Duration::from_millis(16), move || {
        if weak_box.upgrade().is_none() { return glib::ControlFlow::Break; }
        let mut w = widgets.borrow_mut();
        for _ in 0..32 {
            match rx.borrow().try_recv() {
                Ok((id, bytes)) => {
                    if let Some(weak_box) = w.remove(&id) {
                        if let Some(image_container) = weak_box.upgrade() {
                            let gbytes = glib::Bytes::from_owned(bytes);
                            if let Ok(texture) = gtk::gdk::Texture::from_bytes(&gbytes) {
                                while let Some(child) = image_container.first_child() { image_container.remove(&child); }
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
                if let Ok(bytes) = crate::helpers::extractor::extract_cover(&path_clone) {
                    let _ = crate::helpers::thumbnail::generate_all_thumbnails(&bytes, id);
                }
            }).await;
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
