use std::cell::{Cell, RefCell};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

use adw::prelude::*;
use gdk_pixbuf::Pixbuf;
use gtk4::prelude::*;
use gtk4::{self as gtk, gio, glib};
use libadwaita as adw;

use babelcomics_core::helpers::{extraction_registry, extractor};
use crate::ui::run_in_background;

/// Número de thumbnails que se generan en paralelo.
/// Valor bajo a propósito: image::load_from_memory decodifica la imagen completa
/// en RAM antes de escalar (~24-80 MB por página según resolución).
const THUMB_CONCURRENCY: usize = 2;

/// Píxeles RGB crudos de un thumbnail escalado a 160px de ancho.
struct ThumbPixels {
    data: Vec<u8>,
    width: i32,
    height: i32,
    rowstride: i32,
}

/// Genera el thumbnail de una página y devuelve píxeles RGB crudos.
/// Devolver píxeles directamente (sin encode JPEG) evita:
///   - Un encode costoso en el hilo bloqueante.
///   - Un decode costoso en el hilo principal de GTK.
/// La imagen completa se libera explícitamente antes de devolver los píxeles.
async fn make_thumbnail(
    comic_path: String,
    page_name: String,
    pages_dir: PathBuf,
    thumb_path: PathBuf,
) -> anyhow::Result<ThumbPixels> {
    let cached = page_path_in(&page_name, &pages_dir);

    tokio::task::spawn_blocking(move || -> anyhow::Result<ThumbPixels> {
        let rgb = if thumb_path.exists() {
            image::open(&thumb_path)?.into_rgb8()
        } else {
            let img = if cached.exists() {
                image::open(&cached)?
            } else {
                let bytes = extractor::extract_page_to_memory(&comic_path, &page_name)?;
                let img = image::load_from_memory(&bytes)?;
                drop(bytes); // liberar bytes comprimidos antes de escalar
                img
            };

            let thumb = img.resize(160, u32::MAX, image::imageops::FilterType::Triangle);
            drop(img); // liberar imagen completa (~24-80 MB) antes de convertir

            if let Some(parent) = thumb_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            thumb.save_with_format(&thumb_path, image::ImageFormat::Jpeg)?;
            thumb.into_rgb8()
        };
        let width = rgb.width() as i32;
        let height = rgb.height() as i32;
        let rowstride = width * 3;
        let data = rgb.into_raw();

        Ok(ThumbPixels {
            data,
            width,
            height,
            rowstride,
        })
    })
    .await?
}

/// Crea un Pixbuf desde píxeles RGB crudos (sin decode) y lo aplica al widget.
/// Es instantáneo en el hilo principal porque no hay decodificación.
fn apply_thumbnail(img: &gtk::Image, t: ThumbPixels) {
    let pixbuf = Pixbuf::from_bytes(
        &glib::Bytes::from_owned(t.data),
        gdk_pixbuf::Colorspace::Rgb,
        false, // sin canal alpha
        8,     // bits por muestra
        t.width,
        t.height,
        t.rowstride,
    );
    let texture = gtk::gdk::Texture::for_pixbuf(&pixbuf);
    img.set_paintable(Some(&texture));
}

/// Calcula la ruta esperada de una página extraída en `dir`.
fn page_path_in(page_name: &str, dir: &PathBuf) -> PathBuf {
    let file_name = if page_name.chars().all(|c| c.is_ascii_digit()) {
        format!("page-{}.jpg", page_name)
    } else {
        std::path::Path::new(page_name)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
    };
    dir.join(file_name)
}

fn reader_thumb_dir(comic_path: &str) -> PathBuf {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    comic_path.hash(&mut hasher);
    let key = format!("{:016x}", hasher.finish());
    babelcomics_core::helpers::paths::thumbnails_dir()
        .join("comic_pages")
        .join("reader")
        .join(key)
}

fn reader_thumb_path(comic_path: &str, index: usize) -> PathBuf {
    reader_thumb_dir(comic_path).join(format!("page_{}.jpg", index))
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum FitMode {
    Width,
    Height,
    Page,
}

pub struct ReaderWindow {
    win: adw::ApplicationWindow,
    image: gtk::Picture,
    scrolled: gtk::ScrolledWindow,

    // Nombres de páginas ordenados (sin extracción)
    page_names: Rc<RefCell<Vec<String>>>,
    current_index: Rc<Cell<usize>>,

    // Imágenes de la barra lateral: se actualizan conforme se generan thumbnails
    thumbnail_images: Rc<RefCell<Vec<gtk::Image>>>,

    // Marca qué thumbnails ya fueron generados (para no repetir trabajo)
    thumbnail_ready: Rc<RefCell<Vec<bool>>>,

    // UI
    page_entry: gtk::SpinButton,
    total_pages_label: gtk::Label,
    sidebar: adw::OverlaySplitView,
    thumbnail_list: gtk::ListBox,
    stack: gtk::Stack,

    // Estado
    fit_mode: Rc<Cell<FitMode>>,
    /// Directorio para páginas a resolución completa (display + prefetch)
    temp_dir: PathBuf,
    comic_path: Rc<String>,
    updating_entry: Rc<Cell<bool>>,

    // Control de extracción de página principal
    extracting: Rc<Cell<bool>>,
    pending_index: Rc<Cell<Option<usize>>>,

    // Se pone a false al cerrar la ventana; detiene los workers de thumbnails
    alive: Rc<Cell<bool>>,

    // Smart scroll
    scroll_accumulator: Rc<Cell<f64>>,
    last_scroll_time: Rc<Cell<Instant>>,
}

impl ReaderWindow {
    pub fn open(app: &adw::Application, path: &str) {
        let path_buf = PathBuf::from(path);
        let title = path_buf
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Lector".to_string());

        let comic_stem = path_buf
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "comic".to_string());
        let comic_parent = path_buf
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));
        let base_dir = comic_parent.join(".babelcomics").join(&comic_stem);
        let temp_dir = base_dir.join("pages");

        extraction_registry::register(path, &base_dir);

        let win = adw::ApplicationWindow::builder()
            .application(app)
            .title(&format!("{} — Babelcomics", title))
            .default_width(1100)
            .default_height(850)
            .build();

        // --- UI STRUCTURE ---
        let sidebar_view = adw::OverlaySplitView::new();
        sidebar_view.set_collapsed(true);
        sidebar_view.set_sidebar_position(gtk::PackType::Start);

        let toolbar_view = adw::ToolbarView::new();
        let header = adw::HeaderBar::new();

        let btn_sidebar = gtk::ToggleButton::builder()
            .icon_name("sidebar-show-symbolic")
            .tooltip_text("Mostrar páginas (T)")
            .build();
        btn_sidebar
            .bind_property("active", &sidebar_view, "show-sidebar")
            .bidirectional()
            .sync_create()
            .build();
        header.pack_start(&btn_sidebar);

        let nav_box = gtk::Box::builder().spacing(6).margin_start(12).build();
        let page_entry = gtk::SpinButton::builder()
            .numeric(true)
            .width_chars(4)
            .build();
        let total_label = gtk::Label::builder()
            .css_classes(["dim-label", "caption"])
            .build();
        nav_box.append(&page_entry);
        nav_box.append(&total_label);
        header.pack_start(&nav_box);

        let menu_button = gtk::MenuButton::builder()
            .icon_name("preferences-other-symbolic")
            .tooltip_text("Ajustes de visualización")
            .build();
        header.pack_end(&menu_button);

        let btn_fullscreen = gtk::Button::builder()
            .icon_name("view-fullscreen-symbolic")
            .build();
        header.pack_end(&btn_fullscreen);

        toolbar_view.add_top_bar(&header);

        // --- CONTENT ---
        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();

        let scrolled = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hexpand(true)
            .build();
        let image = gtk::Picture::builder().can_shrink(true).build();
        scrolled.set_child(Some(&image));

        let loading_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .build();
        let spinner = adw::Spinner::new();
        loading_box.append(&spinner);
        loading_box.append(&gtk::Label::new(Some("Abriendo comic...")));

        stack.add_named(&loading_box, Some("loading"));
        stack.add_named(&scrolled, Some("reader"));
        toolbar_view.set_content(Some(&stack));
        sidebar_view.set_content(Some(&toolbar_view));

        // --- SIDEBAR ---
        let sidebar_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .width_request(220)
            .css_classes(["background"])
            .build();
        let sidebar_header = gtk::Label::builder()
            .label("Páginas")
            .halign(gtk::Align::Start)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .css_classes(["heading"])
            .build();
        sidebar_box.append(&sidebar_header);

        let thumb_scroll = gtk::ScrolledWindow::builder().vexpand(true).build();
        let thumbnail_list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .build();
        thumb_scroll.set_child(Some(&thumbnail_list));
        sidebar_box.append(&thumb_scroll);
        sidebar_view.set_sidebar(Some(&sidebar_box));

        win.set_content(Some(&sidebar_view));

        let reader = Rc::new(Self {
            win: win.clone(),
            image,
            scrolled,
            page_names: Rc::new(RefCell::new(Vec::new())),
            current_index: Rc::new(Cell::new(0)),
            thumbnail_images: Rc::new(RefCell::new(Vec::new())),
            thumbnail_ready: Rc::new(RefCell::new(Vec::new())),
            page_entry,
            total_pages_label: total_label,
            sidebar: sidebar_view,
            thumbnail_list,
            stack,
            fit_mode: Rc::new(Cell::new(FitMode::Width)),
            temp_dir,
            comic_path: Rc::new(path.to_string()),
            updating_entry: Rc::new(Cell::new(false)),
            extracting: Rc::new(Cell::new(false)),
            pending_index: Rc::new(Cell::new(None)),
            alive: Rc::new(Cell::new(true)),
            scroll_accumulator: Rc::new(Cell::new(0.0)),
            last_scroll_time: Rc::new(Cell::new(Instant::now())),
        });

        reader.setup_logic(&btn_fullscreen, &menu_button);

        let r_cb = reader.clone();
        let path_task = path.to_string();
        run_in_background(
            tokio::runtime::Handle::current(),
            async move { extractor::list_pages(&path_task) },
            move |res| match res {
                Ok(names) => r_cb.on_pages_listed(names),
                Err(_) => r_cb.win.close(),
            },
        );

        win.present();
    }

    fn on_pages_listed(&self, names: Vec<String>) {
        let count = names.len();
        *self.page_names.borrow_mut() = names;
        *self.thumbnail_ready.borrow_mut() = vec![false; count];

        self.page_entry.set_range(1.0, count as f64);
        self.total_pages_label.set_label(&format!("de {}", count));

        self.build_sidebar_placeholders(count);
        self.show_page(0);
        self.start_thumbnail_workers();
    }

    fn build_sidebar_placeholders(&self, count: usize) {
        let mut thumb_images = self.thumbnail_images.borrow_mut();

        for i in 0..count {
            let row = gtk::ListBoxRow::new();
            let box_ = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(4)
                .margin_top(8)
                .margin_bottom(8)
                .build();

            let img = gtk::Image::builder()
                .icon_name("image-x-generic-symbolic")
                .pixel_size(160)
                .build();
            let lbl = gtk::Label::new(Some(&format!("{}", i + 1)));
            lbl.add_css_class("caption");

            box_.append(&img);
            box_.append(&lbl);
            row.set_child(Some(&box_));
            self.thumbnail_list.append(&row);
            thumb_images.push(img);
        }

        let r = self.clone_ref();
        self.thumbnail_list.connect_row_activated(move |_, row| {
            r.show_page(row.index() as usize);
        });
    }

    /// Lanza `THUMB_CONCURRENCY` workers independientes, cada uno procesa su
    /// "lane": worker 0 → páginas 0, 6, 12…; worker 1 → páginas 1, 7, 13…; etc.
    fn start_thumbnail_workers(&self) {
        let count = self.page_names.borrow().len();
        let lanes = THUMB_CONCURRENCY.min(count);
        for lane in 0..lanes {
            self.load_thumbnail_at(lane);
        }
    }

    /// Genera el thumbnail de `index` en background y, al terminar, continúa
    /// con `index + THUMB_CONCURRENCY` (su siguiente turno en la misma lane).
    fn load_thumbnail_at(&self, index: usize) {
        if !self.alive.get() {
            return;
        }

        let count = self.page_names.borrow().len();
        if index >= count {
            return;
        }

        // Si ya está listo (p.ej. al reabrir el sidebar) no repetimos trabajo
        if self.thumbnail_ready.borrow()[index] {
            self.load_thumbnail_at(index + THUMB_CONCURRENCY);
            return;
        }

        let page_name = self.page_names.borrow()[index].clone();
        let comic_path = (*self.comic_path).clone();
        let temp_dir = self.temp_dir.clone();
        let thumb_path = reader_thumb_path(&comic_path, index);
        let r = self.clone_ref();

        run_in_background(
            tokio::runtime::Handle::current(),
            make_thumbnail(comic_path, page_name, temp_dir, thumb_path),
            move |res| {
                if !r.alive.get() {
                    return;
                }
                if let Ok(pixels) = res {
                    r.thumbnail_ready.borrow_mut()[index] = true;
                    let images = r.thumbnail_images.borrow();
                    if let Some(img) = images.get(index) {
                        apply_thumbnail(img, pixels);
                    }
                }
                r.load_thumbnail_at(index + THUMB_CONCURRENCY);
            },
        );
    }

    fn show_page(&self, index: usize) {
        let count = self.page_names.borrow().len();
        if index >= count {
            return;
        }

        self.current_index.set(index);
        self.updating_entry.set(true);
        self.page_entry.set_value((index + 1) as f64);
        self.updating_entry.set(false);

        if let Some(row) = self.thumbnail_list.row_at_index(index as i32) {
            self.thumbnail_list.select_row(Some(&row));
        }

        let page_name = self.page_names.borrow()[index].clone();
        let cached_path = self.page_path_for(&page_name);

        if cached_path.exists() {
            self.display_image(&cached_path);
            self.stack.set_visible_child_name("reader");
            self.prefetch_page(index + 1);
            self.prefetch_page(index + 2);
            return;
        }

        if self.extracting.get() {
            self.pending_index.set(Some(index));
            return;
        }

        self.extracting.set(true);

        let comic_path = (*self.comic_path).clone();
        let temp_dir = self.temp_dir.clone();
        let r = self.clone_ref();
        let requested_index = index;

        run_in_background(
            tokio::runtime::Handle::current(),
            async move { extractor::extract_single_page(&comic_path, &page_name, &temp_dir) },
            move |res| {
                r.extracting.set(false);

                if let Ok(path) = res {
                    if r.current_index.get() == requested_index {
                        r.display_image(&path);
                        r.stack.set_visible_child_name("reader");
                        r.prefetch_page(requested_index + 1);
                        r.prefetch_page(requested_index + 2);
                    }
                }

                if let Some(pending) = r.pending_index.get() {
                    r.pending_index.set(None);
                    r.show_page(pending);
                }
            },
        );
    }

    fn prefetch_page(&self, index: usize) {
        let names = self.page_names.borrow();
        if index >= names.len() {
            return;
        }
        let page_name = names[index].clone();
        drop(names);

        let cached = self.page_path_for(&page_name);
        if cached.exists() {
            return;
        }

        let comic_path = (*self.comic_path).clone();
        let temp_dir = self.temp_dir.clone();

        run_in_background(
            tokio::runtime::Handle::current(),
            async move { extractor::extract_single_page(&comic_path, &page_name, &temp_dir) },
            |_| {},
        );
    }

    fn display_image(&self, path: &PathBuf) {
        self.image.set_file(Some(&gio::File::for_path(path)));
        self.apply_fit_mode();
        self.scrolled.vadjustment().set_value(0.0);
    }

    fn page_path_for(&self, page_name: &str) -> PathBuf {
        page_path_in(page_name, &self.temp_dir)
    }

    fn setup_logic(&self, btn_fs: &gtk::Button, btn_menu: &gtk::MenuButton) {
        let r = self.clone_ref();

        let r_fs = r.clone_ref();
        btn_fs.connect_clicked(move |_| {
            if r_fs.win.is_fullscreen() {
                r_fs.win.unfullscreen();
            } else {
                r_fs.win.fullscreen();
            }
        });

        let r_pe = r.clone_ref();
        self.page_entry.connect_value_changed(move |s| {
            if !r_pe.updating_entry.get() {
                r_pe.show_page(s.value() as usize - 1);
            }
        });

        let r_ss = r.clone_ref();
        let scroll_controller =
            gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        scroll_controller.connect_scroll(move |_, _dx, dy| r_ss.handle_smart_scroll(dy));
        self.scrolled.add_controller(scroll_controller);

        let r_click = r.clone_ref();
        let click_ctrl = gtk::GestureClick::new();
        click_ctrl.connect_pressed(move |_, _, x, _| {
            let width = r_click.win.width() as f64;
            if x < width * 0.3 {
                r_click.prev_page();
            } else if x > width * 0.7 {
                r_click.next_page();
            }
        });
        self.image.add_controller(click_ctrl);

        let shortcut_ctrl = gtk::ShortcutController::new();
        self.add_shortcut(&shortcut_ctrl, "Right|space|Page_Down", move |r| {
            r.next_page()
        });
        self.add_shortcut(&shortcut_ctrl, "Left|BackSpace|Page_Up", move |r| {
            r.prev_page()
        });
        self.add_shortcut(&shortcut_ctrl, "f|F11", move |r| {
            if r.win.is_fullscreen() {
                r.win.unfullscreen();
            } else {
                r.win.fullscreen();
            }
        });
        self.add_shortcut(&shortcut_ctrl, "t", move |r| {
            r.sidebar.set_show_sidebar(!r.sidebar.shows_sidebar());
        });
        self.add_shortcut(&shortcut_ctrl, "Escape", move |r| {
            if r.win.is_fullscreen() {
                r.win.unfullscreen();
            } else {
                r.win.close();
            }
        });
        self.win.add_controller(shortcut_ctrl);

        self.setup_menu(btn_menu);

        let alive_flag = self.alive.clone();
        let base_dir = self
            .temp_dir
            .parent()
            .unwrap_or(&self.temp_dir)
            .to_path_buf();
        self.win.connect_close_request(move |_| {
            alive_flag.set(false);
            extraction_registry::unregister(&base_dir);
            glib::Propagation::Proceed
        });
    }

    fn handle_smart_scroll(&self, dy: f64) -> glib::Propagation {
        let adj = self.scrolled.vadjustment();
        let at_edge = if dy > 0.0 {
            adj.value() >= adj.upper() - adj.page_size() - 5.0
        } else {
            adj.value() <= 5.0
        };

        if at_edge {
            let now = Instant::now();
            let last = self.last_scroll_time.get();
            if now.duration_since(last).as_millis() > 500 {
                self.scroll_accumulator.set(0.0);
            }
            self.last_scroll_time.set(now);

            let acc = self.scroll_accumulator.get() + dy.abs();
            self.scroll_accumulator.set(acc);

            if acc > 3.0 {
                self.scroll_accumulator.set(0.0);
                if dy > 0.0 {
                    self.next_page();
                } else {
                    self.prev_page();
                }
                return glib::Propagation::Stop;
            }
        }
        glib::Propagation::Proceed
    }

    fn setup_menu(&self, btn: &gtk::MenuButton) {
        let menu = gio::Menu::new();
        let section_fit = gio::Menu::new();
        section_fit.append(Some("Ajustar al ancho"), Some("reader.fit_width"));
        section_fit.append(Some("Ajustar a la altura"), Some("reader.fit_height"));
        section_fit.append(Some("Página completa"), Some("reader.fit_page"));
        menu.append_section(Some("Ajuste"), &section_fit);
        btn.set_menu_model(Some(&menu));

        let action_group = gio::SimpleActionGroup::new();
        let r_w = self.clone_ref();
        let r_h = self.clone_ref();
        let r_p = self.clone_ref();

        let action_width = gio::SimpleAction::new("fit_width", None);
        action_width.connect_activate(move |_, _| r_w.set_fit(FitMode::Width));
        action_group.add_action(&action_width);

        let action_height = gio::SimpleAction::new("fit_height", None);
        action_height.connect_activate(move |_, _| r_h.set_fit(FitMode::Height));
        action_group.add_action(&action_height);

        let action_page = gio::SimpleAction::new("fit_page", None);
        action_page.connect_activate(move |_, _| r_p.set_fit(FitMode::Page));
        action_group.add_action(&action_page);

        self.win.insert_action_group("reader", Some(&action_group));
    }

    fn set_fit(&self, mode: FitMode) {
        self.fit_mode.set(mode);
        self.apply_fit_mode();
    }

    fn apply_fit_mode(&self) {
        match self.fit_mode.get() {
            FitMode::Width => self.image.set_content_fit(gtk::ContentFit::ScaleDown),
            FitMode::Height => self.image.set_content_fit(gtk::ContentFit::ScaleDown),
            FitMode::Page => self.image.set_content_fit(gtk::ContentFit::Contain),
        }
    }

    fn next_page(&self) {
        let idx = self.current_index.get();
        if idx + 1 < self.page_names.borrow().len() {
            self.show_page(idx + 1);
        }
    }

    fn prev_page(&self) {
        let idx = self.current_index.get();
        if idx > 0 {
            self.show_page(idx - 1);
        }
    }

    fn add_shortcut<F>(&self, ctrl: &gtk::ShortcutController, trigger: &str, action_fn: F)
    where
        F: Fn(&Self) + 'static,
    {
        let r = self.clone_ref();
        let action = gtk::CallbackAction::new(move |_, _| {
            action_fn(&r);
            glib::Propagation::Stop
        });
        ctrl.add_shortcut(gtk::Shortcut::new(
            Some(gtk::ShortcutTrigger::parse_string(trigger).unwrap()),
            Some(action),
        ));
    }

    fn clone_ref(&self) -> Self {
        Self {
            win: self.win.clone(),
            image: self.image.clone(),
            scrolled: self.scrolled.clone(),
            page_names: self.page_names.clone(),
            current_index: self.current_index.clone(),
            thumbnail_images: self.thumbnail_images.clone(),
            thumbnail_ready: self.thumbnail_ready.clone(),
            page_entry: self.page_entry.clone(),
            total_pages_label: self.total_pages_label.clone(),
            sidebar: self.sidebar.clone(),
            thumbnail_list: self.thumbnail_list.clone(),
            stack: self.stack.clone(),
            fit_mode: self.fit_mode.clone(),
            temp_dir: self.temp_dir.clone(),
            comic_path: self.comic_path.clone(),
            updating_entry: self.updating_entry.clone(),
            extracting: self.extracting.clone(),
            pending_index: self.pending_index.clone(),
            alive: self.alive.clone(),
            scroll_accumulator: self.scroll_accumulator.clone(),
            last_scroll_time: self.last_scroll_time.clone(),
        }
    }
}
