use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{LazyLock, mpsc};
use std::time::Duration;
use tokio::sync::Semaphore;

use gtk4::prelude::*;
use gtk4::{self as gtk, ScrolledWindow, glib};
use libadwaita as adw;
use sqlx::SqlitePool;

use crate::helpers::paths::comic_thumbnail_path;
use crate::helpers::thumbnail::CardSize;
use crate::models::{ComicFilter, ComicbookView};
use crate::repositories::{ComicbookRepository, SetupRepository};
use crate::ui::run_in_background;

/// Widgets pendientes de recibir su thumbnail.
/// Mapa de id_comicbook → (numero, weak_ref al contenedor de imagen).
/// Solo se accede desde el hilo GTK.
type ThumbWidgets = Rc<RefCell<HashMap<i64, (Option<String>, glib::WeakRef<gtk::Box>)>>>;

/// Resultado de leer un thumbnail del disco en background:
/// (id_comicbook, bytes_jpeg_crudos)
/// La decodificación la hace GTK con libjpeg-turbo (mucho más rápido que image crate).
type ThumbResult = (i64, Vec<u8>);

/// Ordered list of comic IDs — same order as WrapBox children.
/// Used to map "selected" CSS cards back to their comic IDs.
type ComicIdRegistry = Rc<RefCell<Vec<i64>>>;

pub fn build(pool: SqlitePool, tab_view: adw::TabView) -> gtk::Widget {
    let toolbar = adw::ToolbarView::new();

    // Barra de búsqueda
    let search_entry = gtk::SearchEntry::builder()
        .placeholder_text("Buscar comics…")
        .hexpand(true)
        .build();

    // Filtros UI
    let filter_popover = gtk::Popover::new();
    let filter_vbox = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    filter_vbox.append(
        &gtk::Label::builder()
            .label("Filtros")
            .halign(gtk::Align::Start)
            .css_classes(["heading"])
            .build(),
    );

    let check_classified = gtk::CheckButton::with_label("Solo clasificados");
    filter_vbox.append(&check_classified);

    let check_unclassified = gtk::CheckButton::with_label("Solo sin clasificar");
    filter_vbox.append(&check_unclassified);

    filter_vbox.append(
        &gtk::Label::builder()
            .label("Calidad mínima")
            .halign(gtk::Align::Start)
            .css_classes(["caption"])
            .build(),
    );
    let scale_quality = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 5.0, 1.0);
    scale_quality.set_draw_value(true);
    filter_vbox.append(&scale_quality);

    filter_popover.set_child(Some(&filter_vbox));

    let filter_btn = gtk::MenuButton::builder()
        .icon_name("emblem-system-symbolic")
        .popover(&filter_popover)
        .tooltip_text("Filtros de búsqueda")
        .build();

    let search_container = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .halign(gtk::Align::Center)
        .build();
    search_container.append(&search_entry);
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

    // ── Atajos de teclado locales (F5 para refrescar) ───────────────────────
    let controller = gtk::ShortcutController::new();
    controller.set_scope(gtk::ShortcutScope::Managed);
    toolbar.add_controller(controller.clone());

    // Grid de comics
    let scrolled = ScrolledWindow::builder()
        .vexpand(true)
        .hexpand(true)
        .build();

    // WrapBox: cada card tiene su width_request fijo, WrapBox las distribuye
    // automáticamente en filas según el espacio disponible — sin límite artificial
    // de columnas ni homogeneous forzado.
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

    // Placeholder cuando no hay comics
    let placeholder = adw::StatusPage::builder()
        .title("Sin comics")
        .description("Añade un directorio en Preferencias y ejecuta el escaneo")
        .icon_name("folder-symbolic")
        .build();

    // Spinner mientras carga
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

    // ── Label de resultados (barra inferior) ────────────────────────────────
    let results_label = gtk::Label::builder()
        .css_classes(["caption", "dim-label"])
        .halign(gtk::Align::Center)
        .margin_top(4)
        .margin_bottom(4)
        .build();
    let results_bar = gtk::CenterBox::builder()
        .center_widget(&results_label)
        .build();
    toolbar.add_bottom_bar(&results_bar);

    // ── Sistema de thumbnails ────────────────────────────────────────────────
    // Canal: tokio tasks (productores) → hilo GTK (consumidor)
    // Los productores leen el archivo y decodifican la imagen en background.
    // El hilo GTK recibe pixels RGBA crudos y crea MemoryTexture (sin I/O).
    let (thumb_tx, thumb_rx) = mpsc::channel::<ThumbResult>();
    let thumb_widgets: ThumbWidgets = Rc::new(RefCell::new(HashMap::new()));

    // Timer GTK que consume resultados y actualiza widgets
    start_thumbnail_consumer(thumb_widgets.clone(), thumb_rx, wrap_box.downgrade());

    // ── Registro de IDs de comics (mismo orden que WrapBox children) ─────────
    let comic_id_registry: ComicIdRegistry = Rc::new(RefCell::new(Vec::new()));

    // ── Menú contextual (click derecho sobre selección) ───────────────────────
    let ctx_popover = gtk::Popover::builder().has_arrow(false).build();
    // Usamos un Button flat en lugar de ActionRow+ListBox para garantizar
    // que connect_clicked funcione sin ambigüedades dentro del Popover.
    let clip_btn_inner = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(8)
        .margin_end(12)
        .build();
    clip_btn_inner.append(
        &gtk::Image::builder()
            .icon_name("find-location-symbolic")
            .pixel_size(16)
            .build(),
    );
    let clip_btn_label_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(1)
        .build();
    clip_btn_label_box.append(
        &gtk::Label::builder()
            .label("Buscar series similares por portada")
            .halign(gtk::Align::Start)
            .build(),
    );
    clip_btn_label_box.append(
        &gtk::Label::builder()
            .label("Catalogación inteligente con CLIP")
            .halign(gtk::Align::Start)
            .css_classes(["caption", "dim-label"])
            .build(),
    );
    clip_btn_inner.append(&clip_btn_label_box);
    let clip_btn = gtk::Button::builder()
        .css_classes(["flat"])
        .child(&clip_btn_inner)
        .build();
    ctx_popover.set_child(Some(&clip_btn));
    // Parentamos al ScrolledWindow, NO al WrapBox.
    // El WrapBox es limpiado con el patrón `while first_child() { remove() }` en
    // cada refresh, lo que llama gtk_widget_unparent() también sobre el popover
    // si fuese hijo del WrapBox — dejándolo sin parent y causando segfault en el
    // siguiente popup(). ScrolledWindow nunca itera sus hijos de ese modo.
    {
        let pop = ctx_popover.clone();
        scrolled.connect_realize(move |sw| {
            // Sólo una vez: si ya tiene parent, no volver a asignarlo.
            if pop.parent().is_none() {
                pop.set_parent(sw);
            }
        });
    }

    // ── Estado de paginación ─────────────────────────────────────────────────
    let offset: Rc<Cell<i64>> = Rc::new(Cell::new(0));
    let generation: Rc<Cell<u64>> = Rc::new(Cell::new(0));
    let loading: Rc<Cell<bool>> = Rc::new(Cell::new(false));
    let all_loaded: Rc<Cell<bool>> = Rc::new(Cell::new(false));
    let page_size: Rc<Cell<i64>> = Rc::new(Cell::new(200));
    let last_clicked: Rc<Cell<i32>> = Rc::new(Cell::new(-1));
    let filter_state: Rc<RefCell<ComicFilter>> = Rc::new(RefCell::new(ComicFilter::default()));
    let total_count: Rc<Cell<i64>> = Rc::new(Cell::new(0));
    let tab_view = tab_view; // ya es owned, solo nombramos para claridad

    // ── Closure que actualiza el label con total / cargados / seleccionados ──
    let actualizar_label: Rc<dyn Fn()> = {
        let label = results_label.clone();
        let total_rc = total_count.clone();
        let offset_rc = offset.clone();
        let wb_weak = wrap_box.downgrade();
        Rc::new(move || {
            let total = total_rc.get();
            if total == 0 {
                label.set_text("");
                return;
            }

            let loaded = offset_rc.get();
            let selected = wb_weak.upgrade().map_or(0, |wb| contar_seleccionados(&wb));

            let mut parts = vec![if total == 1 {
                "1 comic".to_string()
            } else {
                format!("{} comics", total)
            }];
            if loaded < total {
                parts.push(format!("{} cargados", loaded));
            }
            if selected > 0 {
                parts.push(if selected == 1 {
                    "1 seleccionado".to_string()
                } else {
                    format!("{} seleccionados", selected)
                });
            }
            label.set_text(&parts.join(" · "));
        })
    };

    // ── Acción del menú contextual: catalogación CLIP ─────────────────────────
    {
        let wb = wrap_box.clone();
        let registry = comic_id_registry.clone();
        let tv = tab_view.clone();
        let pool_ctx = pool.clone();
        let popover = ctx_popover.clone();
        clip_btn.connect_clicked(move |_| {
            popover.popdown();
            // Collect IDs of all "selected" cards in the same order
            let ids = registry.borrow();
            let mut child = wb.first_child();
            let mut i = 0usize;
            let mut selected: Vec<i64> = Vec::new();
            while let Some(c) = child {
                if c.has_css_class("selected") {
                    if let Some(&id) = ids.get(i) {
                        selected.push(id);
                    }
                }
                child = c.next_sibling();
                i += 1;
            }
            if selected.is_empty() {
                return;
            }
            let page = crate::ui::window::add_tab(
                &tv,
                crate::ui::window::TabKind::CatalogacionInteligente(selected, pool_ctx.clone()),
            );
            tv.set_selected_page(&page);
        });
    }

    // ── Eventos de búsqueda y filtros ────────────────────────────────────────
    let fs_refresh = filter_state.clone();
    let fs1 = filter_state.clone();
    let fs2 = filter_state.clone();
    let fs3 = filter_state.clone();
    let fs4 = filter_state.clone();
    {
        let pool = pool.clone();
        let wrap_box = wrap_box.clone();
        let stack = stack.clone();
        let thumb_widgets_r = thumb_widgets.clone();
        let thumb_tx_r = thumb_tx.clone();
        let (off, load, done, ps, lc, gen_state) = (
            offset.clone(),
            loading.clone(),
            all_loaded.clone(),
            page_size.clone(),
            last_clicked.clone(),
            generation.clone(),
        );

        let tab_view_r = tab_view.clone();
        let registry_r = comic_id_registry.clone();
        let ctx_popover_refresh = ctx_popover.clone();
        let total_count_r = total_count.clone();
        let on_update_r = actualizar_label.clone();
        let refresh_fn = move || {
            while let Some(child) = wrap_box.first_child() {
                wrap_box.remove(&child);
            }
            thumb_widgets_r.borrow_mut().clear();
            registry_r.borrow_mut().clear();

            let new_gen = gen_state.get() + 1;
            gen_state.set(new_gen);

            off.set(0);
            load.set(false);
            done.set(false);
            lc.set(-1);
            total_count_r.set(0);

            let filter = fs_refresh.borrow().clone();
            cargar_pagina(
                &wrap_box,
                &stack,
                &pool,
                &off,
                &load,
                &done,
                &ps,
                &thumb_widgets_r,
                &thumb_tx_r,
                &lc,
                true,
                filter,
                &tab_view_r,
                &registry_r,
                &ctx_popover_refresh,
                new_gen,
                gen_state.clone(),
                &total_count_r,
                &on_update_r,
            );
        };

        let refresh_c1 = Rc::new(refresh_fn);

        let r1 = refresh_c1.clone();
        let se1 = search_entry.clone();
        search_entry.connect_search_changed(move |_| {
            fs1.borrow_mut().query = Some(se1.text().to_string());
            r1();
        });

        let r2 = refresh_c1.clone();
        check_classified.connect_toggled(move |btn| {
            fs2.borrow_mut().clasificado = if btn.is_active() { Some(true) } else { None };
            r2();
        });

        let r3 = refresh_c1.clone();
        check_unclassified.connect_toggled(move |btn| {
            if btn.is_active() {
                fs3.borrow_mut().clasificado = Some(false);
            } else if fs3.borrow_mut().clasificado == Some(false) {
                fs3.borrow_mut().clasificado = None;
            }
            r3();
        });

        let r4 = refresh_c1.clone();
        scale_quality.connect_value_changed(move |s| {
            fs4.borrow_mut().min_calidad = Some(s.value() as i32);
            r4();
        });

        let r5 = refresh_c1.clone();
        // Conectar F5 al controlador de atajos
        let action = gtk::CallbackAction::new(move |_, _| {
            r5();
            glib::Propagation::Stop
        });
        controller.add_shortcut(gtk::Shortcut::new(
            Some(gtk::ShortcutTrigger::parse_string("F5").unwrap()),
            Some(action),
        ));
    }

    // ── Scroll infinito ──────────────────────────────────────────────────────
    {
        let adj = scrolled.vadjustment();
        let (wb_s, stack_s, pool_s) = (wrap_box.clone(), stack.clone(), pool.clone());
        let (off_s, load_s, done_s, ps_s, lc_s, gen_s) = (
            offset.clone(),
            loading.clone(),
            all_loaded.clone(),
            page_size.clone(),
            last_clicked.clone(),
            generation.clone(),
        );
        let tw_s = thumb_widgets.clone();
        let ttx_s = thumb_tx.clone();
        let fs_s = filter_state.clone();
        let tab_view_s = tab_view.clone();
        let registry_s = comic_id_registry.clone();
        let ctx_popover_s = ctx_popover.clone();
        let total_count_s = total_count.clone();
        let on_update_s = actualizar_label.clone();
        adj.connect_value_changed(move |adj| {
            if load_s.get() || done_s.get() {
                return;
            }
            if adj.value() + adj.page_size() >= adj.upper() - 800.0 {
                let filter = fs_s.borrow().clone();
                let current_gen = gen_s.get();
                cargar_pagina(
                    &wb_s,
                    &stack_s,
                    &pool_s,
                    &off_s,
                    &load_s,
                    &done_s,
                    &ps_s,
                    &tw_s,
                    &ttx_s,
                    &lc_s,
                    false,
                    filter,
                    &tab_view_s,
                    &registry_s,
                    &ctx_popover_s,
                    current_gen,
                    gen_s.clone(),
                    &total_count_s,
                    &on_update_s,
                );
            }
        });
    }

    // ── Carga inicial ────────────────────────────────────────────────────────
    cargar_pagina(
        &wrap_box,
        &stack,
        &pool,
        &offset,
        &loading,
        &all_loaded,
        &page_size,
        &thumb_widgets,
        &thumb_tx,
        &last_clicked,
        true,
        ComicFilter::default(),
        &tab_view,
        &comic_id_registry,
        &ctx_popover,
        0,
        generation.clone(),
        &total_count,
        &actualizar_label,
    );

    toolbar.upcast()
}

// ---------------------------------------------------------------------------

/// Timer GTK que drena el canal de resultados y actualiza widgets.
/// Decodifica con gdk::Texture::from_bytes (libjpeg-turbo, <1ms por thumb pequeño).
/// Procesa hasta MAX_PER_TICK por tick para no bloquear el event loop.
fn start_thumbnail_consumer(
    widgets: ThumbWidgets,
    rx: mpsc::Receiver<ThumbResult>,
    weak_box: glib::WeakRef<adw::WrapBox>,
) {
    let rx = Rc::new(RefCell::new(rx));
    // libjpeg-turbo decodifica un JPEG de 400px en <0.5ms;
    // 32 por tick a 16ms = headroom de ~15ms para el resto del event loop.
    const MAX_PER_TICK: usize = 32;

    glib::timeout_add_local(Duration::from_millis(16), move || {
        if weak_box.upgrade().is_none() {
            return glib::ControlFlow::Break;
        }

        let rx = rx.borrow();
        let mut w = widgets.borrow_mut();

        for _ in 0..MAX_PER_TICK {
            match rx.try_recv() {
                Ok((id, bytes)) => {
                    // Ignorar resultados de pages antiguas (id ya no está en el mapa)
                    let Some((numero, weak_box)) = w.remove(&id) else {
                        continue;
                    };
                    let Some(image_container) = weak_box.upgrade() else {
                        continue;
                    };

                    // gdk::Texture::from_bytes usa libjpeg-turbo internamente:
                    // decode JPEG en <1ms sin salir del hilo GTK.
                    let gbytes = glib::Bytes::from_owned(bytes);
                    let Ok(texture) = gtk::gdk::Texture::from_bytes(&gbytes) else {
                        continue;
                    };

                    while let Some(child) = image_container.first_child() {
                        image_container.remove(&child);
                    }
                    image_container.append(&cover_frame_from_texture(&texture, numero.as_deref()));
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return glib::ControlFlow::Break,
            }
        }

        glib::ControlFlow::Continue
    });
}

/// Máximo de generaciones on-demand simultáneas.
/// Evita saturar el disco cuando muchos comics son visibles a la vez.
static THUMB_SEM: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(8));

/// Lanza una tokio task que asegura que el thumbnail exista.
/// Si no existe, lo genera inmediatamente (Prioridad UI).
/// Si ya existe, simplemente lo lee y lo envía.
fn schedule_thumbnail(id: i64, path: String, tx: mpsc::Sender<ThumbResult>, size: CardSize) {
    tokio::runtime::Handle::current().spawn(async move {
        let _permit = THUMB_SEM.acquire().await.unwrap();
        let thumb_path = comic_thumbnail_path(id, size);

        // Si no existe, lo generamos on-demand y esperamos a que termine.
        if !thumb_path.exists() {
            let path_clone = path.clone();
            let _ = tokio::task::spawn_blocking(move || {
                if let Ok(bytes) = crate::helpers::extractor::extract_cover(&path_clone) {
                    let _ = crate::helpers::thumbnail::generate_all_thumbnails(&bytes, id);
                }
            })
            .await;
        }

        // El spawn_blocking ya terminó: el archivo existe o falló — lectura directa.
        if let Ok(bytes) = tokio::fs::read(&thumb_path).await {
            let _ = tx.send((id, bytes));
        }
    });
}

// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn cargar_pagina(
    flow: &adw::WrapBox,
    stack: &gtk::Stack,
    pool: &SqlitePool,
    offset: &Rc<Cell<i64>>,
    loading: &Rc<Cell<bool>>,
    all_loaded: &Rc<Cell<bool>>,
    page_size: &Rc<Cell<i64>>,
    thumb_widgets: &ThumbWidgets,
    thumb_tx: &mpsc::Sender<ThumbResult>,
    last_clicked: &Rc<Cell<i32>>,
    is_first: bool,
    filter: ComicFilter,
    tab_view: &adw::TabView,
    registry_done: &ComicIdRegistry,
    ctx_popover: &gtk::Popover,
    current_gen: u64,
    gen_rc: Rc<Cell<u64>>,
    total_count: &Rc<Cell<i64>>,
    on_update: &Rc<dyn Fn()>,
) {
    if loading.get() {
        return;
    }
    loading.set(true);

    if is_first {
        stack.set_visible_child_name("loading");
    }

    let current_offset = offset.get();
    let current_page_size = page_size.get();
    let pool_task = pool.clone();
    let flow_done: adw::WrapBox = flow.clone();
    let stack_done = stack.clone();
    let offset_done = offset.clone();
    let loading_done = loading.clone();
    let all_done = all_loaded.clone();
    let ps_done = page_size.clone();
    let tw_done = thumb_widgets.clone();
    let ttx_done = thumb_tx.clone();
    let lc_done = last_clicked.clone();
    let pool_ui = pool.clone();
    let tab_view_done = tab_view.clone();
    let registry_done = registry_done.clone();
    let ctx_popover_done = ctx_popover.clone();
    let gen_check = gen_rc;
    let total_count_done = total_count.clone();
    let on_update_done = on_update.clone();
    let on_update_idle = on_update.clone();

    run_in_background(
        tokio::runtime::Handle::current(),
        async move {
            let setup = SetupRepository::new(&pool_task)
                .get()
                .await
                .unwrap_or_default();

            let limit = if is_first {
                setup.items_por_pagina.max(50)
            } else {
                current_page_size
            };

            let card_size = CardSize::from_db(setup.thumbnail_size);

            let repo = ComicbookRepository::new(&pool_task);
            let comics = repo
                .get_page(limit, current_offset, false, Some(&filter))
                .await
                .unwrap_or_default();

            let total = if is_first {
                repo.count_filtered(&filter).await.unwrap_or(0)
            } else {
                0
            };

            (comics, limit, card_size, total)
        },
        move |(comics, limit, card_size, total)| {
            loading_done.set(false);

            // Si la búsqueda ya no es la actual, ignoramos resultados
            if gen_check.get() != current_gen {
                return;
            }

            ps_done.set(limit);
            let received = comics.len() as i64;

            if is_first && received == 0 {
                stack_done.set_visible_child_name("empty");
                total_count_done.set(0);
                on_update_done();
                all_done.set(true);
                return;
            }

            if is_first {
                total_count_done.set(total);
            }

            if received < limit {
                all_done.set(true);
            }

            offset_done.set(current_offset + received);
            on_update_done();

            if is_first {
                stack_done.set_visible_child_name("grid");
            }

            // Agregar cards en lotes idle para no bloquear el hilo GTK
            let comics = Rc::new(comics);
            let idx = Rc::new(RefCell::new(0usize));
            const BATCH: usize = 25;

            glib::idle_add_local(move || {
                // Volver a comprobar en cada lote por si el usuario cambió de búsqueda mientras
                if gen_check.get() != current_gen {
                    return glib::ControlFlow::Break;
                }

                let start = *idx.borrow();
                let end = (start + BATCH).min(comics.len());

                for comic in &comics[start..end] {
                    let (card, img_weak) = build_card(comic, card_size);
                    adjuntar_gesto_seleccion(
                        &card,
                        &flow_done,
                        &lc_done,
                        comic.id_comicbook,
                        pool_ui.clone(),
                        tab_view_done.clone(),
                        ctx_popover_done.clone(),
                        on_update_idle.clone(),
                    );
                    flow_done.append(&card);
                    registry_done.borrow_mut().push(comic.id_comicbook);

                    tw_done
                        .borrow_mut()
                        .insert(comic.id_comicbook, (comic.numero.clone(), img_weak));
                    schedule_thumbnail(
                        comic.id_comicbook,
                        comic.path.clone(),
                        ttx_done.clone(),
                        card_size,
                    );
                }

                *idx.borrow_mut() = end;
                if end >= comics.len() {
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            });
        },
    );
}

/// Adjunta un GestureClick a la card para selección:
/// - Click simple  → selecciona solo esta card
/// - Ctrl+Click    → toggle esta card
/// - Shift+Click   → selecciona rango desde la última clickeada
/// La selección se marca con la clase CSS "selected" en cada card.
fn adjuntar_gesto_seleccion(
    card: &gtk::Widget,
    wrap_box: &adw::WrapBox,
    last_clicked: &Rc<Cell<i32>>,
    comic_id: i64,
    pool: SqlitePool,
    tab_view: adw::TabView,
    ctx_popover: gtk::Popover,
    on_selection_change: Rc<dyn Fn()>,
) {
    // Right-click: asegura que la card quede seleccionada y muestra el menú contextual
    {
        let right_click = gtk::GestureClick::new();
        right_click.set_button(3);
        let card_weak_r = card.downgrade();
        let wb_weak_r = wrap_box.downgrade();
        let lc_r = last_clicked.clone();
        let pop = ctx_popover.clone();
        let on_sel_r = on_selection_change.clone();
        right_click.connect_pressed(move |g, _, x, y| {
            let (Some(wb), Some(card)) = (wb_weak_r.upgrade(), card_weak_r.upgrade()) else {
                return;
            };
            // Si el popover perdió su parent por alguna razón, no intentar mostrarlo.
            let Some(pop_parent) = pop.parent() else {
                return;
            };
            if !card.has_css_class("selected") {
                wb_deselect_all(&wb);
                card.add_css_class("selected");
                lc_r.set(wb_child_index(&wb, &card));
                on_sel_r();
            }
            // Traducir coordenadas de la card al espacio del parent del popover.
            let (wx, wy) = card
                .translate_coordinates(&pop_parent, x, y)
                .unwrap_or((x, y));
            let rect = gtk::gdk::Rectangle::new(wx as i32, wy as i32, 1, 1);
            pop.set_pointing_to(Some(&rect));
            pop.popup();
            g.set_state(gtk::EventSequenceState::Claimed);
        });
        card.add_controller(right_click);
    }

    let gesture = gtk::GestureClick::new();
    let wb_weak = wrap_box.downgrade();
    let card_weak = card.downgrade();
    let lc = last_clicked.clone();

    gesture.connect_pressed(move |g, n_press, _, _| {
        let (Some(wb), Some(card)) = (wb_weak.upgrade(), card_weak.upgrade()) else {
            return;
        };

        // Doble-click: abrir solapa de detalle
        if n_press >= 2 {
            let mods = g.current_event_state();
            let ctrl = mods.contains(gtk::gdk::ModifierType::CONTROL_MASK);

            let page = crate::ui::window::add_tab(
                &tab_view,
                crate::ui::window::TabKind::ComicDetail(comic_id, pool.clone()),
            );

            // Si NO está pulsado Ctrl, cambiamos a la nueva pestaña.
            // Si SÍ está pulsado Ctrl, la dejamos en segundo plano.
            if !ctrl {
                tab_view.set_selected_page(&page);
            }

            g.set_state(gtk::EventSequenceState::Claimed);
            return;
        }

        // Click simple: selección
        let mods = g.current_event_state();
        let ctrl = mods.contains(gtk::gdk::ModifierType::CONTROL_MASK);
        let shift = mods.contains(gtk::gdk::ModifierType::SHIFT_MASK);

        let idx = wb_child_index(&wb, &card);

        if shift && lc.get() >= 0 {
            wb_deselect_all(&wb);
            let last = lc.get();
            let (a, b) = if last <= idx {
                (last, idx)
            } else {
                (idx, last)
            };
            wb_select_range(&wb, a, b);
        } else if ctrl {
            if card.has_css_class("selected") {
                card.remove_css_class("selected");
            } else {
                card.add_css_class("selected");
                lc.set(idx);
            }
        } else {
            wb_deselect_all(&wb);
            card.add_css_class("selected");
            lc.set(idx);
        }

        on_selection_change();
        g.set_state(gtk::EventSequenceState::Claimed);
    });

    card.add_controller(gesture);
}

fn contar_seleccionados(wb: &adw::WrapBox) -> usize {
    let mut n = 0usize;
    let mut child = wb.first_child();
    while let Some(c) = child {
        if c.has_css_class("selected") {
            n += 1;
        }
        child = c.next_sibling();
    }
    n
}

fn wb_child_index(wb: &adw::WrapBox, target: &gtk::Widget) -> i32 {
    let mut child = wb.first_child();
    let mut i = 0i32;
    while let Some(c) = child {
        if &c == target {
            return i;
        }
        child = c.next_sibling();
        i += 1;
    }
    -1
}

fn wb_deselect_all(wb: &adw::WrapBox) {
    let mut child = wb.first_child();
    while let Some(c) = child {
        c.remove_css_class("selected");
        child = c.next_sibling();
    }
}

fn wb_select_range(wb: &adw::WrapBox, from: i32, to: i32) {
    let mut child = wb.first_child();
    let mut i = 0i32;
    while let Some(c) = child {
        if i >= from && i <= to {
            c.add_css_class("selected");
        }
        child = c.next_sibling();
        i += 1;
    }
}

// ---------------------------------------------------------------------------

/// Construye la card de un comic con placeholder.
/// Devuelve (widget, weak_ref al contenedor de imagen) para registrar
/// la carga async del thumbnail.
fn build_card(
    comic: &ComicbookView,
    card_size: CardSize,
) -> (gtk::Widget, glib::WeakRef<gtk::Box>) {
    let (cw, ch) = card_size.dims();

    // La card ya no tiene ancho fijo. Se adapta a su contenido.
    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(0)
        .margin_top(4)
        .margin_bottom(4)
        .focusable(true)
        .halign(gtk::Align::Center)
        .hexpand(false)
        .css_classes(["card"])
        .build();

    // Contenedor con ALTURA FIJA, pero ancho flexible.
    let image_container = gtk::Box::builder()
        .height_request(ch as i32)
        .halign(gtk::Align::Center)
        .hexpand(false)
        .overflow(gtk::Overflow::Hidden)
        .css_classes(["comic-cover-container"])
        .build();

    // Placeholder: altura fija ch, ancho libre
    let img = gtk::Image::builder()
        .icon_name("image-x-generic-symbolic")
        .pixel_size((cw / 3) as i32)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    img.set_opacity(0.3);
    image_container.append(&img);

    let img_weak = image_container.downgrade();

    let info_box = create_info_box(comic);

    card.append(&image_container);
    card.append(&info_box);

    (card.upcast(), img_weak)
}

fn create_info_box(comic: &ComicbookView) -> gtk::Box {
    let info_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .margin_top(8)
        .margin_start(4)
        .margin_end(4)
        .margin_bottom(6)
        .halign(gtk::Align::Center)
        .build();

    let title_str = comic_title(comic);
    let title_label = gtk::Label::builder()
        .label(&title_str)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .lines(2)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .justify(gtk::Justification::Center)
        .halign(gtk::Align::Center)
        .hexpand(false)
        .max_width_chars(12)
        .css_classes(["caption"])
        .build();
    info_box.append(&title_label);

    let extra_info = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .halign(gtk::Align::Center)
        .build();

    if let Some(num) = &comic.numero {
        let num_label = gtk::Label::builder()
            .label(&format!("#{}", num))
            .css_classes(["dim-label", "caption"])
            .build();
        extra_info.append(&num_label);
    }

    let status_box = gtk::Box::builder().spacing(4).build();
    let (icon_name, label_text, css) = if comic.titulo.is_some() {
        ("view-reveal-symbolic", "Clasificado", "success")
    } else {
        ("view-conceal-symbolic", "Sin clasificar", "warning")
    };
    status_box.append(
        &gtk::Image::builder()
            .icon_name(icon_name)
            .css_classes([css])
            .pixel_size(12)
            .build(),
    );
    status_box.append(
        &gtk::Label::builder()
            .label(label_text)
            .css_classes(["caption", css])
            .build(),
    );
    extra_info.append(&status_box);

    if let Some(calidad) = &comic.calidad {
        if let Ok(val) = calidad.parse::<i32>() {
            let stars = "★".repeat(val as usize);
            let label = gtk::Label::builder()
                .label(&stars)
                .css_classes(["caption", "accent"])
                .build();
            extra_info.append(&label);
        }
    }

    info_box.append(&extra_info);
    info_box
}

fn comic_title(comic: &ComicbookView) -> String {
    if let Some(titulo) = &comic.titulo {
        return titulo.clone();
    }
    std::path::Path::new(&comic.path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| comic.path.clone())
}

/// Crea un overlay con la imagen y el número de issue.
/// La imagen define el ancho basándose en su altura fija.
fn cover_frame_from_texture(texture: &gtk::gdk::Texture, numero: Option<&str>) -> gtk::Widget {
    let picture = gtk::Picture::for_paintable(texture);
    picture.set_content_fit(gtk::ContentFit::Contain);
    picture.set_can_shrink(true);
    picture.add_css_class("cover-image");
    let overlay = gtk::Overlay::builder().child(&picture).build();

    if let Some(num) = numero {
        let num_tag = gtk::Label::builder()
            .label(num)
            .halign(gtk::Align::End)
            .valign(gtk::Align::Start)
            .css_classes(["issue-number-overlay"])
            .build();
        overlay.add_overlay(&num_tag);
    }

    overlay.upcast()
}
