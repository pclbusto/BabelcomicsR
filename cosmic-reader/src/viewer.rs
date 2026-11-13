use crate::options::{Fit, Guided, ReaderOptions, ScaleFilter};
use crate::progress::ProgressTracker;
use crate::thumb_cache;
use cosmic::Element;
use cosmic::iced::mouse;
use cosmic::iced::widget::Stack;
use cosmic::iced::widget::canvas::{self, Canvas, Frame, Geometry, Path, Program, Stroke};
use cosmic::iced::widget::image::{FilterMethod, Handle};
use cosmic::iced::widget::scrollable::{Direction, Scrollbar};
use cosmic::iced::{Color, ContentFit, Length, Point, Rectangle, Renderer, Size};
use cosmic::widget::{Column, Image, Row, button, text};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::process::Command;
use zip::ZipArchive;

const THUMBNAIL_BATCH_SIZE: usize = 12;
const THUMBNAIL_WIDTH: u32 = 180;
const THUMBNAIL_HEIGHT: u32 = 220;
/// Ancho reservado a la barra de desplazamiento en el ajuste a lo ancho.
const SCROLLBAR_RESERVE: f32 = 14.0;

/// Identificador del área desplazable de la página, para volver arriba al
/// cambiar de página.
pub fn page_scroll_id() -> cosmic::widget::Id {
    cosmic::widget::Id::new("reader-page")
}

/// Identificador del panel lateral de miniaturas.
pub fn sidebar_scroll_id() -> cosmic::widget::Id {
    cosmic::widget::Id::new("reader-sidebar")
}

const SIDEBAR_WIDTH: f32 = 150.0;
const SIDEBAR_THUMB_WIDTH: f32 = 110.0;
const SIDEBAR_THUMB_HEIGHT: f32 = 134.0;
const THUMBNAIL_JPEG_QUALITY: u8 = 85;

#[derive(Clone, Debug)]
pub struct ThumbnailRequest {
    comic_path: PathBuf,
    pages: Vec<(usize, String)>,
    filter: ScaleFilter,
}

#[derive(Clone, Debug)]
pub struct ThumbnailData {
    pub page_index: usize,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// La misma miniatura codificada, para guardarla en `babel-thb`.
    pub jpeg: Vec<u8>,
}

pub enum ArchiveFormat {
    Zip(ZipArchive<File>),
    Rar,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Reading,
    Editor,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Panel {
    pub id: usize,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum HandlePos {
    TopLeft,
    TopCenter,
    TopRight,
    Left,
    Right,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

#[derive(Clone, Debug)]
pub enum DragState {
    None,
    Creating {
        start: Point,
    },
    Moving {
        panel_idx: usize,
        original: Panel,
        start: Point,
    },
    Resizing {
        panel_idx: usize,
        handle: HandlePos,
        original: Panel,
        start: Point,
    },
}

impl Default for DragState {
    fn default() -> Self {
        DragState::None
    }
}

#[derive(Default)]
pub struct EditorState {
    pub drag: DragState,
    pub current_drag: Option<Rectangle>,
}

fn get_image_bounds(bounds: Rectangle, img_size: Option<(u32, u32)>) -> Rectangle {
    if let Some((w, h)) = img_size {
        let img_w = w as f32;
        let img_h = h as f32;
        let scale = (bounds.width / img_w).min(bounds.height / img_h);
        let scaled_w = img_w * scale;
        let scaled_h = img_h * scale;
        // Tanto `cursor.position_in(bounds)` como `Frame` trabajan en el
        // sistema local del Canvas. Sumar bounds.x/y mezclaba coordenadas de
        // ventana con coordenadas locales y guardaba el recorte desplazado.
        let x = (bounds.width - scaled_w) / 2.0;
        let y = (bounds.height - scaled_h) / 2.0;
        Rectangle::new(Point::new(x, y), Size::new(scaled_w, scaled_h))
    } else {
        Rectangle::new(Point::ORIGIN, bounds.size())
    }
}

fn screen_to_img(p: Point, bounds: Rectangle, img_size: Option<(u32, u32)>) -> Point {
    if let Some((w, _)) = img_size {
        let img_b = get_image_bounds(bounds, img_size);
        let scale = img_b.width / (w as f32);
        Point::new((p.x - img_b.x) / scale, (p.y - img_b.y) / scale)
    } else {
        p
    }
}

fn img_to_screen(p: Point, bounds: Rectangle, img_size: Option<(u32, u32)>) -> Point {
    if let Some((w, _)) = img_size {
        let img_b = get_image_bounds(bounds, img_size);
        let scale = img_b.width / (w as f32);
        Point::new(p.x * scale + img_b.x, p.y * scale + img_b.y)
    } else {
        p
    }
}

fn get_handles(rect: Rectangle) -> [(HandlePos, Rectangle); 8] {
    let s = 10.0;
    let half = s / 2.0;
    [
        (
            HandlePos::TopLeft,
            Rectangle::new(Point::new(rect.x - half, rect.y - half), Size::new(s, s)),
        ),
        (
            HandlePos::TopCenter,
            Rectangle::new(
                Point::new(rect.center_x() - half, rect.y - half),
                Size::new(s, s),
            ),
        ),
        (
            HandlePos::TopRight,
            Rectangle::new(
                Point::new(rect.x + rect.width - half, rect.y - half),
                Size::new(s, s),
            ),
        ),
        (
            HandlePos::Left,
            Rectangle::new(
                Point::new(rect.x - half, rect.center_y() - half),
                Size::new(s, s),
            ),
        ),
        (
            HandlePos::Right,
            Rectangle::new(
                Point::new(rect.x + rect.width - half, rect.center_y() - half),
                Size::new(s, s),
            ),
        ),
        (
            HandlePos::BottomLeft,
            Rectangle::new(
                Point::new(rect.x - half, rect.y + rect.height - half),
                Size::new(s, s),
            ),
        ),
        (
            HandlePos::BottomCenter,
            Rectangle::new(
                Point::new(rect.center_x() - half, rect.y + rect.height - half),
                Size::new(s, s),
            ),
        ),
        (
            HandlePos::BottomRight,
            Rectangle::new(
                Point::new(rect.x + rect.width - half, rect.y + rect.height - half),
                Size::new(s, s),
            ),
        ),
    ]
}

pub struct ComicViewer {
    pub file_path: Option<PathBuf>,
    pub pages: Vec<String>,
    pub current_page: usize,
    pub current_image_bytes: Option<Vec<u8>>,
    pub current_image_size: Option<(u32, u32)>,
    pub current_dynamic_image: Option<::image::DynamicImage>,
    pub error: Option<String>,
    pub archive: Option<ArchiveFormat>,

    // Editor State
    pub mode: AppMode,
    pub page_panels: HashMap<String, Vec<Panel>>,
    pub selected_panel: Option<usize>,

    // Guided View State
    pub show_guided_prompt: bool,
    pub guided_view: bool,
    pub current_panel_index: usize,

    // Reading progress
    progress_tracker: Option<ProgressTracker>,
    pub progress_error: Option<String>,

    // Page overview
    pub show_page_grid: bool,
    pub show_sidebar: bool,
    /// Miniaturas codificadas por índice de página, listas para `babel-thb`.
    thumbnail_jpegs: HashMap<usize, Vec<u8>>,
    /// El cómic ya tiene una caché válida: no hace falta volver a escribirla.
    thumbnail_cache_valid: bool,
    page_thumbnails: HashMap<usize, Handle>,
    thumbnail_next_index: usize,
    thumbnail_loading: bool,
    pub thumbnail_error: Option<String>,

    pub options: ReaderOptions,
}

impl ComicViewer {
    pub fn new() -> Self {
        Self {
            file_path: None,
            pages: Vec::new(),
            current_page: 0,
            current_image_bytes: None,
            current_image_size: None,
            current_dynamic_image: None,
            error: None,
            archive: None,
            mode: AppMode::Reading,
            page_panels: HashMap::new(),
            selected_panel: None,
            show_guided_prompt: false,
            guided_view: false,
            current_panel_index: 0,
            progress_tracker: None,
            progress_error: None,
            show_page_grid: false,
            show_sidebar: true,
            thumbnail_jpegs: HashMap::new(),
            thumbnail_cache_valid: false,
            page_thumbnails: HashMap::new(),
            thumbnail_next_index: 0,
            thumbnail_loading: false,
            thumbnail_error: None,
            options: ReaderOptions::default(),
        }
    }

    /// Decide entre preguntar, activar o ignorar la lectura guiada según las
    /// preferencias, cuando el cómic tiene viñetas definidas.
    fn apply_guided_preference(&mut self) {
        let available = self.mode == AppMode::Reading && self.has_guided_panels();
        self.current_panel_index = 0;
        match self.options.guided {
            Guided::Ask => {
                self.show_guided_prompt = available;
                self.guided_view = false;
            }
            Guided::Always => {
                self.show_guided_prompt = false;
                self.guided_view = available;
            }
            Guided::Never => {
                self.show_guided_prompt = false;
                self.guided_view = false;
            }
        }
    }

    fn image_filter_method(&self) -> FilterMethod {
        if self.options.filter == ScaleFilter::Nearest {
            FilterMethod::Nearest
        } else {
            FilterMethod::Linear
        }
    }

    pub fn toggle_mode(&mut self) {
        match self.mode {
            AppMode::Reading => {
                self.mode = AppMode::Editor;
                self.guided_view = false;
                self.show_guided_prompt = false;
                self.current_panel_index = 0;
            }
            AppMode::Editor => {
                self.mode = AppMode::Reading;
                self.apply_guided_preference();
            }
        }
    }

    pub fn set_guided_view(&mut self, enabled: bool) {
        self.guided_view = enabled && self.mode == AppMode::Reading && self.has_guided_panels();
        self.show_guided_prompt = false;
        self.current_panel_index = 0;
    }

    fn has_guided_panels(&self) -> bool {
        self.page_panels.values().any(|panels| !panels.is_empty())
    }

    pub fn configure_progress_file(&mut self, progress_path: PathBuf) {
        let Some(comic_path) = self.file_path.clone() else {
            return;
        };

        match ProgressTracker::open(progress_path, comic_path, self.pages.len()) {
            Ok(tracker) => {
                let resume_page = tracker.resume_page_index();
                self.progress_tracker = Some(tracker);
                self.progress_error = None;

                if !self.pages.is_empty() && resume_page != self.current_page {
                    self.current_page = resume_page;
                    self.selected_panel = None;
                    self.load_current_page();
                }
                self.record_progress();
            }
            Err(error) => {
                self.progress_tracker = None;
                self.progress_error = Some(error);
            }
        }
    }

    fn record_progress(&mut self) {
        if self.current_image_bytes.is_none() {
            return;
        }
        if let Some(tracker) = self.progress_tracker.as_mut() {
            self.progress_error = tracker.visit(self.current_page).err();
        }
    }

    pub fn toggle_page_grid(&mut self) {
        self.show_page_grid = !self.show_page_grid;

        if self.show_page_grid {
            self.thumbnail_error = None;
            self.thumbnail_loading = false;
            self.thumbnail_next_index = 0;

            if let Some(image) = self
                .current_dynamic_image
                .as_ref()
                .filter(|_| !self.page_thumbnails.contains_key(&self.current_page))
            {
                let thumbnail = image
                    .resize(
                        THUMBNAIL_WIDTH,
                        THUMBNAIL_HEIGHT,
                        self.options.filter.filter_type(),
                    )
                    .to_rgba8();
                self.page_thumbnails.insert(
                    self.current_page,
                    Handle::from_rgba(thumbnail.width(), thumbnail.height(), thumbnail.into_raw()),
                );
            }
        }
    }

    pub fn toggle_sidebar(&mut self) {
        self.show_sidebar = !self.show_sidebar;
    }

    /// Carga `babel-thb` si el cómic la trae y corresponde exactamente a sus páginas.
    fn load_thumbnail_cache(&mut self) {
        let bytes = match self.archive.as_mut() {
            Some(ArchiveFormat::Zip(archive)) => thumb_cache::read_from_zip(archive),
            Some(ArchiveFormat::Rar) => self
                .file_path
                .as_deref()
                .and_then(thumb_cache::read_from_rar),
            None => None,
        };
        let Some(cached) =
            bytes.and_then(|bytes| thumb_cache::parse(&bytes, THUMBNAIL_WIDTH, THUMBNAIL_HEIGHT))
        else {
            return;
        };
        if cached.len() != self.pages.len() {
            return;
        }
        let indexes: HashMap<&str, usize> = self
            .pages
            .iter()
            .enumerate()
            .map(|(index, name)| (name.as_str(), index))
            .collect();
        let mut jpegs = HashMap::with_capacity(cached.len());
        for thumbnail in cached {
            let Some(&index) = indexes.get(thumbnail.page_name.as_str()) else {
                return;
            };
            jpegs.insert(index, thumbnail.jpeg);
        }
        if jpegs.len() != self.pages.len() {
            return;
        }
        for (index, jpeg) in &jpegs {
            self.page_thumbnails
                .insert(*index, Handle::from_bytes(jpeg.clone()));
        }
        self.thumbnail_jpegs = jpegs;
        self.thumbnail_cache_valid = true;
    }

    /// Escribe `babel-thb` dentro del cómic si se generaron todas las
    /// miniaturas y el archivo no tenía ya una caché válida. Devuelve `true` si
    /// escribió algo.
    pub fn save_thumbnail_cache(&mut self) -> Result<bool, String> {
        if self.thumbnail_cache_valid
            || self.pages.is_empty()
            || self.thumbnail_jpegs.len() != self.pages.len()
        {
            return Ok(false);
        }
        let Some(path) = self.file_path.clone() else {
            return Ok(false);
        };
        let mut thumbnails = Vec::with_capacity(self.pages.len());
        for (index, name) in self.pages.iter().enumerate() {
            let Some(jpeg) = self.thumbnail_jpegs.get(&index) else {
                return Ok(false);
            };
            thumbnails.push((name.as_str(), jpeg.as_slice()));
        }
        let data = thumb_cache::serialize(THUMBNAIL_WIDTH, THUMBNAIL_HEIGHT, &thumbnails);
        // Se marca antes de escribir: si falla no se reintenta en cada cierre.
        self.thumbnail_cache_valid = true;
        thumb_cache::write_into_archive(&path, &data)?;
        if let Some(tracker) = self.progress_tracker.as_mut() {
            tracker.refresh_after_archive_change()?;
        }
        Ok(true)
    }

    pub fn take_thumbnail_request(&mut self) -> Option<ThumbnailRequest> {
        if !(self.show_page_grid || self.show_sidebar)
            || self.thumbnail_loading
            || self.thumbnail_error.is_some()
        {
            return None;
        }

        loop {
            if self.thumbnail_next_index >= self.pages.len() {
                return None;
            }

            let mut pages = Vec::with_capacity(THUMBNAIL_BATCH_SIZE);
            while self.thumbnail_next_index < self.pages.len() && pages.len() < THUMBNAIL_BATCH_SIZE
            {
                let page_index = self.thumbnail_next_index;
                self.thumbnail_next_index += 1;
                if !self.thumbnail_jpegs.contains_key(&page_index) {
                    pages.push((page_index, self.pages[page_index].clone()));
                }
            }

            if !pages.is_empty() {
                self.thumbnail_loading = true;
                return Some(ThumbnailRequest {
                    comic_path: self.file_path.clone()?,
                    pages,
                    filter: self.options.filter,
                });
            }
        }
    }

    pub fn apply_thumbnail_batch(&mut self, result: Result<Vec<ThumbnailData>, String>) {
        self.thumbnail_loading = false;
        match result {
            Ok(thumbnails) => {
                for thumbnail in thumbnails {
                    self.thumbnail_jpegs
                        .insert(thumbnail.page_index, thumbnail.jpeg);
                    self.page_thumbnails.insert(
                        thumbnail.page_index,
                        Handle::from_rgba(thumbnail.width, thumbnail.height, thumbnail.rgba),
                    );
                }
            }
            Err(error) => self.thumbnail_error = Some(error),
        }
    }

    pub fn generate_thumbnail_batch(
        request: ThumbnailRequest,
    ) -> Result<Vec<ThumbnailData>, String> {
        let extension = request
            .comic_path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        if extension == "cbr" || extension == "rar" {
            let mut thumbnails = Vec::with_capacity(request.pages.len());
            for (page_index, page_name) in request.pages {
                let output = Command::new("unrar")
                    .args([
                        "p",
                        "-inul",
                        request.comic_path.to_str().unwrap_or(""),
                        &page_name,
                    ])
                    .output()
                    .map_err(|error| format!("No se pudo ejecutar unrar: {error}"))?;
                if output.status.success() {
                    if let Some(thumbnail) =
                        make_thumbnail(page_index, &output.stdout, request.filter)
                    {
                        thumbnails.push(thumbnail);
                    }
                }
            }
            Ok(thumbnails)
        } else {
            let file = File::open(&request.comic_path).map_err(|error| {
                format!("No se pudo abrir {}: {error}", request.comic_path.display())
            })?;
            let mut archive = ZipArchive::new(file)
                .map_err(|error| format!("No se pudo abrir el archivo ZIP: {error}"))?;
            let mut thumbnails = Vec::with_capacity(request.pages.len());
            for (page_index, page_name) in request.pages {
                let Ok(mut page) = archive.by_name(&page_name) else {
                    continue;
                };
                let mut bytes = Vec::new();
                if page.read_to_end(&mut bytes).is_ok() {
                    if let Some(thumbnail) = make_thumbnail(page_index, &bytes, request.filter) {
                        thumbnails.push(thumbnail);
                    }
                }
            }
            Ok(thumbnails)
        }
    }

    pub fn select_page(&mut self, page_index: usize) {
        if page_index >= self.pages.len() {
            return;
        }

        self.current_page = page_index;
        self.current_panel_index = 0;
        self.selected_panel = None;
        self.show_page_grid = false;
        self.load_current_page();
        self.record_progress();
    }

    pub fn save_panels(&mut self) {
        if let Some(path) = &self.file_path {
            let json_path = path.with_extension("json");
            match serde_json::to_string_pretty(&self.page_panels) {
                Ok(json) => {
                    if let Err(e) = std::fs::write(&json_path, json) {
                        self.error =
                            Some(format!("Error guardando {}: {}", json_path.display(), e));
                    } else {
                        self.error = Some(format!("Paneles guardados en {}", json_path.display()));
                    }
                }
                Err(e) => {
                    self.error = Some(format!("Error serializando paneles: {}", e));
                }
            }
        }
    }

    pub fn load_file(&mut self, path: PathBuf) {
        self.file_path = Some(path.clone());
        self.error = None;
        self.archive = None;
        self.progress_tracker = None;
        self.progress_error = None;
        self.show_page_grid = false;
        self.page_thumbnails.clear();
        self.thumbnail_jpegs.clear();
        self.thumbnail_cache_valid = false;
        self.thumbnail_next_index = 0;
        self.thumbnail_loading = false;
        self.thumbnail_error = None;
        self.page_panels.clear();
        self.selected_panel = None;
        self.show_guided_prompt = false;
        self.guided_view = false;

        // Load sidecar json if exists
        let json_path = path.with_extension("json");
        if json_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&json_path) {
                if let Ok(mut panels) =
                    serde_json::from_str::<HashMap<String, Vec<Panel>>>(&content)
                {
                    for page_panels in panels.values_mut() {
                        page_panels.sort_by_key(|panel| panel.id);
                    }
                    self.page_panels = panels;
                    self.apply_guided_preference();
                }
            }
        }

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        if ext == "cbr" || ext == "rar" {
            self.load_rar(path);
        } else {
            self.load_zip(path);
        }
    }

    fn load_zip(&mut self, path: PathBuf) {
        match File::open(&path) {
            Ok(file) => match ZipArchive::new(file) {
                Ok(mut archive) => {
                    let mut pages = Vec::new();
                    for i in 0..archive.len() {
                        if let Ok(file) = archive.by_index(i) {
                            let name = file.name().to_string();
                            let lower = name.to_lowercase();
                            if lower.ends_with(".jpg")
                                || lower.ends_with(".jpeg")
                                || lower.ends_with(".png")
                                || lower.ends_with(".webp")
                            {
                                pages.push(name);
                            }
                        }
                    }

                    pages.sort();
                    self.pages = pages;
                    self.current_page = 0;
                    self.archive = Some(ArchiveFormat::Zip(archive));
                    self.load_thumbnail_cache();
                    self.load_current_page();
                }
                Err(e) => self.error = Some(format!("Error al abrir el CBZ (Zip): {}", e)),
            },
            Err(e) => self.error = Some(format!("Error al leer el archivo CBZ: {}", e)),
        }
    }

    fn load_rar(&mut self, path: PathBuf) {
        let output = Command::new("unrar")
            .args(["lb", path.to_str().unwrap_or("")])
            .output();

        match output {
            Ok(out) => {
                let listing = String::from_utf8_lossy(&out.stdout);
                let mut pages = Vec::new();
                for line in listing.lines() {
                    let lower = line.to_lowercase();
                    if lower.ends_with(".jpg")
                        || lower.ends_with(".jpeg")
                        || lower.ends_with(".png")
                        || lower.ends_with(".webp")
                    {
                        pages.push(line.to_string());
                    }
                }

                if pages.is_empty() {
                    self.error = Some("No se encontraron imágenes en el CBR".to_string());
                } else {
                    pages.sort();
                    self.pages = pages;
                    self.current_page = 0;
                    self.archive = Some(ArchiveFormat::Rar);
                    self.load_thumbnail_cache();
                    self.load_current_page();
                }
            }
            Err(e) => {
                self.error = Some(format!(
                    "Error ejecutando 'unrar'. Verifica que está instalado: {}",
                    e
                ))
            }
        }
    }

    pub fn next_page(&mut self) {
        if self.show_guided_prompt || self.show_page_grid || self.pages.is_empty() {
            return;
        }

        if self.mode == AppMode::Reading && self.guided_view {
            let page_name = &self.pages[self.current_page];
            if let Some(panels) = self.page_panels.get(page_name) {
                if !panels.is_empty() && self.current_panel_index + 1 < panels.len() {
                    self.current_panel_index += 1;
                    return;
                }
            }
        }

        if !self.pages.is_empty() && self.current_page + 1 < self.pages.len() {
            self.current_page += 1;
            self.selected_panel = None;
            self.load_current_page();
            self.record_progress();
        }
    }

    pub fn previous_page(&mut self) {
        if self.show_guided_prompt || self.show_page_grid {
            return;
        }

        if self.mode == AppMode::Reading && self.guided_view {
            if self.current_panel_index > 0 {
                self.current_panel_index -= 1;
                return;
            }
        }

        if self.current_page > 0 {
            self.current_page -= 1;
            self.selected_panel = None;
            self.load_current_page();

            if self.mode == AppMode::Reading && self.guided_view {
                let page_name = &self.pages[self.current_page];
                if let Some(panels) = self.page_panels.get(page_name) {
                    if !panels.is_empty() {
                        self.current_panel_index = panels.len() - 1;
                    }
                }
            }
            self.record_progress();
        }
    }

    fn load_current_page(&mut self) {
        self.current_image_bytes = None;
        self.current_image_size = None;
        self.current_dynamic_image = None;
        if self.pages.is_empty() {
            self.error = Some("El cómic no tiene imágenes válidas.".to_string());
            return;
        }

        let page_name = self.pages[self.current_page].clone();

        let decode_image =
            |bytes: &Vec<u8>| -> (Option<(u32, u32)>, Option<::image::DynamicImage>) {
                if let Ok(img) = ::image::load_from_memory(bytes) {
                    (Some((img.width(), img.height())), Some(img))
                } else {
                    (None, None)
                }
            };

        self.current_panel_index = 0;

        match self.archive.as_mut() {
            Some(ArchiveFormat::Zip(archive)) => match archive.by_name(&page_name) {
                Ok(mut file) => {
                    let mut buffer = Vec::new();
                    if let Err(e) = file.read_to_end(&mut buffer) {
                        self.error = Some(format!("Error leyendo la página ZIP: {}", e));
                    } else {
                        let (size, dyn_img) = decode_image(&buffer);
                        self.current_image_size = size;
                        self.current_dynamic_image = dyn_img;
                        self.current_image_bytes = Some(buffer);
                    }
                }
                Err(e) => self.error = Some(format!("No se encontró la página en el ZIP: {}", e)),
            },
            Some(ArchiveFormat::Rar) => {
                if let Some(path) = &self.file_path {
                    let output = Command::new("unrar")
                        .args(["p", "-inul", path.to_str().unwrap_or(""), &page_name])
                        .output();

                    match output {
                        Ok(out) if out.status.success() => {
                            let (size, dyn_img) = decode_image(&out.stdout);
                            self.current_image_size = size;
                            self.current_dynamic_image = dyn_img;
                            self.current_image_bytes = Some(out.stdout);
                        }
                        Ok(_) => self.error = Some("Falló la extracción con unrar".to_string()),
                        Err(e) => self.error = Some(format!("Error ejecutando unrar: {}", e)),
                    }
                }
            }
            None => {}
        }
    }

    pub fn add_panel(&mut self, panel: Panel) {
        if !self.pages.is_empty() {
            let page_name = self.pages[self.current_page].clone();
            let entry = self.page_panels.entry(page_name).or_insert_with(Vec::new);
            entry.push(panel);
            self.selected_panel = Some(entry.len() - 1);
        }
    }

    pub fn update_panel(&mut self, index: usize, panel: Panel) {
        if !self.pages.is_empty() {
            let page_name = &self.pages[self.current_page];
            if let Some(panels) = self.page_panels.get_mut(page_name) {
                if index < panels.len() {
                    panels[index] = panel;
                }
            }
        }
    }

    pub fn select_panel(&mut self, index: Option<usize>) {
        self.selected_panel = index;
    }

    pub fn delete_selected_panel(&mut self) {
        if self.mode == AppMode::Editor {
            if let Some(index) = self.selected_panel {
                if !self.pages.is_empty() {
                    let page_name = &self.pages[self.current_page];
                    if let Some(panels) = self.page_panels.get_mut(page_name) {
                        if index < panels.len() {
                            panels.remove(index);
                            // Reorder IDs
                            for (i, panel) in panels.iter_mut().enumerate() {
                                panel.id = i + 1;
                            }
                            self.selected_panel = None;
                        }
                    }
                }
            }
        }
    }

    fn guided_panel_handle(&self) -> Option<Handle> {
        let page_name = self.pages.get(self.current_page)?;
        let panel = self
            .page_panels
            .get(page_name)?
            .get(self.current_panel_index)?;
        let image = self.current_dynamic_image.as_ref()?;

        if !panel.x.is_finite()
            || !panel.y.is_finite()
            || !panel.width.is_finite()
            || !panel.height.is_finite()
            || panel.width <= 0.0
            || panel.height <= 0.0
        {
            return None;
        }

        let image_width = image.width() as f32;
        let image_height = image.height() as f32;
        let left = panel.x.clamp(0.0, image_width).floor() as u32;
        let top = panel.y.clamp(0.0, image_height).floor() as u32;
        let right = (panel.x + panel.width).clamp(0.0, image_width).ceil() as u32;
        let bottom = (panel.y + panel.height).clamp(0.0, image_height).ceil() as u32;

        if right <= left || bottom <= top {
            return None;
        }

        let cropped = image.crop_imm(left, top, right - left, bottom - top);
        let rgba = cropped.to_rgba8();
        Some(Handle::from_rgba(
            rgba.width(),
            rgba.height(),
            rgba.into_raw(),
        ))
    }

    /// Página en modo lectura, respetando el ajuste elegido. La lectura guiada
    /// siempre muestra la viñeta completa.
    fn reading_page<'a, Message: 'a + Clone + 'static>(
        &'a self,
        bytes: &[u8],
    ) -> Element<'a, Message> {
        let filter_method = self.image_filter_method();
        let guided_handle = self
            .guided_view
            .then(|| self.guided_panel_handle())
            .flatten();
        let fit = if guided_handle.is_some() {
            Fit::Page
        } else {
            self.options.fit
        };
        let handle = guided_handle.unwrap_or_else(|| Handle::from_bytes(bytes.to_vec()));

        let contain = |handle: Handle| -> Element<'a, Message> {
            Image::new(handle)
                .width(Length::Fill)
                .height(Length::Fill)
                .content_fit(ContentFit::Contain)
                .filter_method(filter_method)
                .into()
        };
        let Some((image_width, image_height)) = self
            .current_image_size
            .filter(|(width, height)| *width > 0 && *height > 0)
        else {
            return contain(handle);
        };
        if fit == Fit::Page {
            return contain(handle);
        }

        cosmic::widget::responsive(move |size| {
            let ratio = image_width as f32 / image_height as f32;
            let (width, height, direction) = match fit {
                Fit::Width => {
                    let width = (size.width - SCROLLBAR_RESERVE).max(1.0);
                    (
                        width,
                        width / ratio,
                        Direction::Vertical(Scrollbar::default()),
                    )
                }
                Fit::Height => {
                    let mut height = size.height;
                    if height * ratio > size.width {
                        height -= SCROLLBAR_RESERVE;
                    }
                    let height = height.max(1.0);
                    (
                        height * ratio,
                        height,
                        Direction::Horizontal(Scrollbar::default()),
                    )
                }
                _ => (
                    image_width as f32,
                    image_height as f32,
                    Direction::Both {
                        vertical: Scrollbar::default(),
                        horizontal: Scrollbar::default(),
                    },
                ),
            };
            let page = Image::new(handle.clone())
                .width(Length::Fixed(width))
                .height(Length::Fixed(height))
                .content_fit(ContentFit::Fill)
                .filter_method(filter_method);
            // Centra las páginas más chicas que la ventana. El contenido de un
            // Scrollable no puede usar Fill en el eje que se desplaza.
            let frame = cosmic::widget::container(page)
                .width(Length::Fixed(width.max(size.width - SCROLLBAR_RESERVE)))
                .height(Length::Fixed(height.max(size.height - SCROLLBAR_RESERVE)))
                .align_x(cosmic::iced::alignment::Horizontal::Center)
                .align_y(cosmic::iced::alignment::Vertical::Center);
            cosmic::iced::widget::Scrollable::new(frame)
                .id(page_scroll_id())
                .direction(direction)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        })
        .into()
    }

    pub fn view<'a, Message: 'a + Clone + 'static>(
        &'a self,
        on_add: impl Fn(Panel) -> Message + 'a,
        on_update: impl Fn(usize, Panel) -> Message + 'a,
        on_select: impl Fn(Option<usize>) -> Message + 'a,
        on_guided: impl Fn(bool) -> Message + 'a,
        on_select_page: impl Fn(usize) -> Message + 'a,
        on_toggle_page_grid: Message,
    ) -> Element<'a, Message> {
        if self.show_page_grid {
            let loaded = self.page_thumbnails.len().min(self.pages.len());
            let mut header = Row::new()
                .spacing(12)
                .push(
                    text(format!(
                        "Páginas · {loaded}/{} miniaturas",
                        self.pages.len()
                    ))
                    .size(20),
                )
                .push(button::standard("Cerrar").on_press(on_toggle_page_grid));
            if let Some(error) = &self.thumbnail_error {
                header = header.push(text(error));
            }

            let overview = cosmic::widget::responsive(move |size| {
                const CARD_WIDTH: f32 = 170.0;
                const GAP: f32 = 12.0;
                let columns = (((size.width + GAP) / (CARD_WIDTH + GAP)).floor() as usize).max(1);
                let mut grid = cosmic::widget::Grid::new()
                    .column_spacing(GAP as u16)
                    .row_spacing(GAP as u16);

                for page_index in 0..self.pages.len() {
                    let page_label = if page_index == self.current_page {
                        format!("Página {} · actual", page_index + 1)
                    } else {
                        format!("Página {}", page_index + 1)
                    };

                    let preview: Element<'a, Message> =
                        if let Some(handle) = self.page_thumbnails.get(&page_index) {
                            button::image(handle.clone())
                                .width(Length::Fixed(160.0))
                                .height(Length::Fixed(180.0))
                                .selected(page_index == self.current_page)
                                .on_press(on_select_page(page_index))
                                .into()
                        } else {
                            cosmic::widget::container(text("Cargando…"))
                                .width(Length::Fixed(160.0))
                                .height(Length::Fixed(180.0))
                                .align_x(cosmic::iced::alignment::Horizontal::Center)
                                .align_y(cosmic::iced::alignment::Vertical::Center)
                                .into()
                        };

                    let card = Column::new()
                        .width(Length::Fixed(CARD_WIDTH))
                        .height(Length::Shrink)
                        .spacing(6)
                        .push(preview)
                        .push(text(page_label));
                    grid = grid.push(card);
                    if (page_index + 1) % columns == 0 {
                        grid = grid.insert_row();
                    }
                }

                cosmic::widget::scrollable(grid).into()
            });

            return Column::new()
                .spacing(12)
                .padding(12)
                .push(header)
                .push(overview)
                .into();
        }

        let mut content = Column::new().spacing(10);

        if self.show_guided_prompt {
            content = content.push(
                Row::new()
                    .spacing(10)
                    .push(
                        text("Este cómic tiene una secuencia de viñetas. ¿Usar lectura guiada?")
                            .size(18),
                    )
                    .push(button::suggested("Sí, activar").on_press(on_guided(true)))
                    .push(button::standard("No, lectura normal").on_press(on_guided(false))),
            );
        }

        if let Some(error) = &self.progress_error {
            content = content.push(text(format!("Progreso: {error}")));
        }

        if let Some(err) = &self.error {
            content = content.push(text(err).size(20));
        } else if let Some(bytes) = &self.current_image_bytes {
            if self.mode == AppMode::Reading {
                content = content.push(self.reading_page(bytes));
            } else {
                let page_name = &self.pages[self.current_page];
                let image = Image::new(Handle::from_bytes(bytes.clone()))
                    .width(cosmic::iced::Length::Fill)
                    .height(cosmic::iced::Length::Fill)
                    .content_fit(ContentFit::Contain);
                let canvas = Canvas::new(EditorCanvas {
                    viewer: self,
                    page_name,
                    on_add_panel: Box::new(on_add),
                    on_update_panel: Box::new(on_update),
                    on_select_panel: Box::new(on_select),
                })
                .width(cosmic::iced::Length::Fill)
                .height(cosmic::iced::Length::Fill);

                // La imagen y las anotaciones deben ser capas de widget distintas.
                // El renderer de Canvas agrupa las imágenes después de la geometría
                // vectorial, aunque draw_image se invoque primero, y por eso podía
                // ocultar los rectángulos. Stack garantiza el orden de composición:
                // imagen debajo, canvas interactivo encima.
                let editor = Stack::new()
                    .push(image)
                    .push(canvas)
                    .width(cosmic::iced::Length::Fill)
                    .height(cosmic::iced::Length::Fill);

                content = content.push(editor);
            }

            let mode_text = if self.mode == AppMode::Reading {
                "Lectura"
            } else {
                "Editor"
            };
            let guided_text = if self.guided_view {
                let page_name = &self.pages[self.current_page];
                match self.page_panels.get(page_name).map(Vec::len) {
                    Some(count) if count > 0 => {
                        format!(
                            " (Guiada: viñeta {}/{})",
                            self.current_panel_index + 1,
                            count
                        )
                    }
                    _ => " (Guiada: página completa)".to_string(),
                }
            } else {
                String::new()
            };
            let status = format!(
                "Página {} / {} | Modo: {}{}",
                self.current_page + 1,
                self.pages.len(),
                mode_text,
                guided_text
            );
            content = content.push(text(status));
        } else if let Some(path) = &self.file_path {
            content = content.push(text(format!("Cargando: {}", path.display())));
        } else {
            content = content.push(text(
                "Ningún cómic seleccionado. Pasa la ruta como argumento.",
            ));
        }

        if self.show_sidebar && !self.pages.is_empty() {
            Row::new()
                .spacing(8)
                .push(self.sidebar(on_select_page))
                .push(content.width(Length::Fill))
                .into()
        } else {
            content.into()
        }
    }

    /// Panel lateral con las miniaturas de todas las páginas.
    fn sidebar<'a, Message: 'a + Clone + 'static>(
        &'a self,
        on_select_page: impl Fn(usize) -> Message + 'a,
    ) -> Element<'a, Message> {
        let mut list = Column::new()
            .spacing(8)
            .padding([8, 14, 8, 8])
            .align_x(cosmic::iced::Alignment::Center);
        for page_index in 0..self.pages.len() {
            let current = page_index == self.current_page;
            let preview: Element<'a, Message> =
                if let Some(handle) = self.page_thumbnails.get(&page_index) {
                    button::image(handle.clone())
                        .width(Length::Fixed(SIDEBAR_THUMB_WIDTH))
                        .height(Length::Fixed(SIDEBAR_THUMB_HEIGHT))
                        .selected(current)
                        .on_press(on_select_page(page_index))
                        .into()
                } else {
                    button::custom(
                        cosmic::widget::container(text::caption("…"))
                            .width(Length::Fixed(SIDEBAR_THUMB_WIDTH))
                            .height(Length::Fixed(SIDEBAR_THUMB_HEIGHT))
                            .align_x(cosmic::iced::alignment::Horizontal::Center)
                            .align_y(cosmic::iced::alignment::Vertical::Center),
                    )
                    .selected(current)
                    .on_press(on_select_page(page_index))
                    .into()
                };
            let label = if current {
                text::caption_heading(format!("{}", page_index + 1))
            } else {
                text::caption(format!("{}", page_index + 1))
            };
            list = list.push(
                Column::new()
                    .spacing(2)
                    .align_x(cosmic::iced::Alignment::Center)
                    .push(preview)
                    .push(label),
            );
        }
        cosmic::iced::widget::Scrollable::new(list)
            .id(sidebar_scroll_id())
            .width(Length::Fixed(SIDEBAR_WIDTH))
            .height(Length::Fill)
            .into()
    }

    /// Posición relativa (0..1) de la página actual dentro del panel lateral.
    pub fn sidebar_offset(&self) -> f32 {
        if self.pages.len() <= 1 {
            0.0
        } else {
            self.current_page as f32 / (self.pages.len() - 1) as f32
        }
    }
}

fn make_thumbnail(page_index: usize, bytes: &[u8], filter: ScaleFilter) -> Option<ThumbnailData> {
    let image = ::image::load_from_memory(bytes).ok()?;
    let thumbnail = image.resize(THUMBNAIL_WIDTH, THUMBNAIL_HEIGHT, filter.filter_type());
    let mut jpeg = Vec::new();
    ::image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, THUMBNAIL_JPEG_QUALITY)
        .encode_image(&thumbnail.to_rgb8())
        .ok()?;
    let thumbnail = thumbnail.to_rgba8();
    Some(ThumbnailData {
        page_index,
        width: thumbnail.width(),
        height: thumbnail.height(),
        jpeg,
        rgba: thumbnail.into_raw(),
    })
}

pub struct EditorCanvas<'a, Message> {
    pub viewer: &'a ComicViewer,
    pub page_name: &'a str,
    pub on_add_panel: Box<dyn Fn(Panel) -> Message + 'a>,
    pub on_update_panel: Box<dyn Fn(usize, Panel) -> Message + 'a>,
    pub on_select_panel: Box<dyn Fn(Option<usize>) -> Message + 'a>,
}

impl<'a, Message> Program<Message, cosmic::Theme, Renderer> for EditorCanvas<'a, Message> {
    type State = EditorState;

    fn update(
        &self,
        state: &mut Self::State,
        event: &cosmic::iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let cursor_position = match cursor.position_in(bounds) {
            Some(p) => p,
            None => return None,
        };

        let img_size = self.viewer.current_image_size;
        let img_p = screen_to_img(cursor_position, bounds, img_size);

        match event {
            cosmic::iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let panels = self
                    .viewer
                    .page_panels
                    .get(self.page_name)
                    .cloned()
                    .unwrap_or_default();

                // 1. Check handles of selected panel
                if let Some(sel_idx) = self.viewer.selected_panel {
                    if let Some(panel) = panels.get(sel_idx) {
                        let top_left =
                            img_to_screen(Point::new(panel.x, panel.y), bounds, img_size);
                        let bottom_right = img_to_screen(
                            Point::new(panel.x + panel.width, panel.y + panel.height),
                            bounds,
                            img_size,
                        );
                        let screen_rect = Rectangle::new(
                            top_left,
                            Size::new(bottom_right.x - top_left.x, bottom_right.y - top_left.y),
                        );

                        for (pos, handle_rect) in get_handles(screen_rect) {
                            if handle_rect.contains(cursor_position) {
                                state.drag = DragState::Resizing {
                                    panel_idx: sel_idx,
                                    handle: pos,
                                    original: panel.clone(),
                                    start: img_p,
                                };
                                return Some(canvas::Action::request_redraw().and_capture());
                            }
                        }
                    }
                }

                // 2. Check inside any panel
                for (i, panel) in panels.iter().enumerate().rev() {
                    let rect = Rectangle::new(
                        Point::new(panel.x, panel.y),
                        Size::new(panel.width, panel.height),
                    );
                    if rect.contains(img_p) {
                        state.drag = DragState::Moving {
                            panel_idx: i,
                            original: panel.clone(),
                            start: img_p,
                        };
                        return Some(
                            canvas::Action::publish((self.on_select_panel)(Some(i))).and_capture(),
                        );
                    }
                }

                // 3. Create new panel
                state.drag = DragState::Creating { start: img_p };
                state.current_drag = Some(Rectangle::new(img_p, Size::new(0.0, 0.0)));

                if self.viewer.selected_panel.is_some() {
                    Some(canvas::Action::publish((self.on_select_panel)(None)).and_capture())
                } else {
                    Some(canvas::Action::request_redraw().and_capture())
                }
            }
            cosmic::iced::Event::Mouse(mouse::Event::CursorMoved { .. }) => match &state.drag {
                DragState::Creating { start } => {
                    let x = start.x.min(img_p.x);
                    let y = start.y.min(img_p.y);
                    let w = (start.x - img_p.x).abs();
                    let h = (start.y - img_p.y).abs();
                    state.current_drag = Some(Rectangle::new(Point::new(x, y), Size::new(w, h)));
                    Some(canvas::Action::request_redraw().and_capture())
                }
                DragState::Moving {
                    panel_idx,
                    original,
                    start,
                } => {
                    let mut updated = original.clone();
                    updated.x += img_p.x - start.x;
                    updated.y += img_p.y - start.y;
                    Some(
                        canvas::Action::publish((self.on_update_panel)(*panel_idx, updated))
                            .and_capture(),
                    )
                }
                DragState::Resizing {
                    panel_idx,
                    handle,
                    original,
                    start,
                } => {
                    let mut updated = original.clone();
                    let dx = img_p.x - start.x;
                    let dy = img_p.y - start.y;

                    match handle {
                        HandlePos::TopLeft => {
                            updated.x += dx;
                            updated.width -= dx;
                            updated.y += dy;
                            updated.height -= dy;
                        }
                        HandlePos::TopCenter => {
                            updated.y += dy;
                            updated.height -= dy;
                        }
                        HandlePos::TopRight => {
                            updated.width += dx;
                            updated.y += dy;
                            updated.height -= dy;
                        }
                        HandlePos::Left => {
                            updated.x += dx;
                            updated.width -= dx;
                        }
                        HandlePos::Right => {
                            updated.width += dx;
                        }
                        HandlePos::BottomLeft => {
                            updated.x += dx;
                            updated.width -= dx;
                            updated.height += dy;
                        }
                        HandlePos::BottomCenter => {
                            updated.height += dy;
                        }
                        HandlePos::BottomRight => {
                            updated.width += dx;
                            updated.height += dy;
                        }
                    }

                    if updated.width < 10.0 {
                        updated.width = 10.0;
                    }
                    if updated.height < 10.0 {
                        updated.height = 10.0;
                    }

                    Some(
                        canvas::Action::publish((self.on_update_panel)(*panel_idx, updated))
                            .and_capture(),
                    )
                }
                DragState::None => None,
            },
            cosmic::iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                let action = match &state.drag {
                    DragState::Creating { .. } => {
                        if let Some(rect) = state.current_drag.take() {
                            if rect.width > 10.0 && rect.height > 10.0 {
                                let existing = self
                                    .viewer
                                    .page_panels
                                    .get(self.page_name)
                                    .map(|v| v.len())
                                    .unwrap_or(0);
                                let panel = Panel {
                                    id: existing + 1,
                                    x: rect.x,
                                    y: rect.y,
                                    width: rect.width,
                                    height: rect.height,
                                };
                                Some(
                                    canvas::Action::publish((self.on_add_panel)(panel))
                                        .and_capture(),
                                )
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                    _ => None,
                };

                state.drag = DragState::None;

                action.or_else(|| Some(canvas::Action::request_redraw().and_capture()))
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _theme: &cosmic::Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        // Se reconstruye la capa en cada frame para que al cambiar de página
        // nunca se reutilicen los rectángulos almacenados de la página anterior.
        let mut frame = Frame::new(renderer, bounds.size());
        let img_size = self.viewer.current_image_size;
        let fill_color = Color::from_rgba(0.4, 0.6, 0.8, 0.5); // Celeste medio gris
        let stroke_color = Color::from_rgb(0.2, 0.4, 0.8);

        if let Some(panels) = self.viewer.page_panels.get(self.page_name) {
            for (i, panel) in panels.iter().enumerate() {
                let top_left = img_to_screen(Point::new(panel.x, panel.y), bounds, img_size);
                let bottom_right = img_to_screen(
                    Point::new(panel.x + panel.width, panel.y + panel.height),
                    bounds,
                    img_size,
                );
                let rect = Rectangle::new(
                    top_left,
                    Size::new(bottom_right.x - top_left.x, bottom_right.y - top_left.y),
                );

                let path = Path::rectangle(rect.position(), rect.size());
                frame.fill(&path, fill_color);
                frame.stroke(
                    &path,
                    Stroke::default().with_color(stroke_color).with_width(2.0),
                );

                frame.fill_text(canvas::Text {
                    content: panel.id.to_string(),
                    position: Point::new(rect.x + 5.0, rect.y + 5.0),
                    color: Color::WHITE,
                    size: cosmic::iced::Pixels(20.0).into(),
                    ..Default::default()
                });

                if Some(i) == self.viewer.selected_panel {
                    for (_, handle_rect) in get_handles(rect) {
                        let handle_path =
                            Path::rectangle(handle_rect.position(), handle_rect.size());
                        frame.fill(&handle_path, Color::WHITE);
                        frame.stroke(
                            &handle_path,
                            Stroke::default().with_color(stroke_color).with_width(2.0),
                        );
                    }
                }
            }
        }

        if let Some(img_rect) = state.current_drag {
            let top_left = img_to_screen(Point::new(img_rect.x, img_rect.y), bounds, img_size);
            let bottom_right = img_to_screen(
                Point::new(img_rect.x + img_rect.width, img_rect.y + img_rect.height),
                bounds,
                img_size,
            );
            let rect = Rectangle::new(
                top_left,
                Size::new(bottom_right.x - top_left.x, bottom_right.y - top_left.y),
            );

            let path = Path::rectangle(rect.position(), rect.size());
            frame.fill(&path, fill_color);
            frame.stroke(
                &path,
                Stroke::default().with_color(stroke_color).with_width(2.0),
            );
        }

        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod coordinate_tests {
    use super::*;

    #[test]
    fn image_coordinates_ignore_the_canvas_window_offset() {
        let bounds = Rectangle::new(Point::new(35.0, 80.0), Size::new(800.0, 600.0));
        let image_size = Some((1000, 2000));
        let image_bounds = get_image_bounds(bounds, image_size);

        assert!((image_bounds.x - 250.0).abs() < f32::EPSILON);
        assert!(image_bounds.y.abs() < f32::EPSILON);

        let image_point = Point::new(240.0, 750.0);
        let screen_point = img_to_screen(image_point, bounds, image_size);
        let round_trip = screen_to_img(screen_point, bounds, image_size);
        assert!((round_trip.x - image_point.x).abs() < 0.001);
        assert!((round_trip.y - image_point.y).abs() < 0.001);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn write_test_comic(path: &std::path::Path, pages: usize) {
        let mut writer = zip::ZipWriter::new(File::create(path).unwrap());
        for index in 0..pages {
            let image =
                ::image::RgbImage::from_pixel(60, 90, ::image::Rgb([index as u8 * 40, 80, 120]));
            let mut png = Vec::new();
            image
                .write_to(
                    &mut std::io::Cursor::new(&mut png),
                    ::image::ImageFormat::Png,
                )
                .unwrap();
            writer
                .start_file(format!("{index:02}.png"), SimpleFileOptions::default())
                .unwrap();
            writer.write_all(&png).unwrap();
        }
        writer.finish().unwrap();
    }

    fn generate_all_thumbnails(viewer: &mut ComicViewer) {
        while let Some(request) = viewer.take_thumbnail_request() {
            viewer.apply_thumbnail_batch(ComicViewer::generate_thumbnail_batch(request));
        }
    }

    #[test]
    fn thumbnail_cache_is_saved_and_reused() {
        let directory =
            std::env::temp_dir().join(format!("babel-viewer-test-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("comic.cbz");
        write_test_comic(&path, 3);
        let modified_before = std::fs::metadata(&path).unwrap().modified().unwrap();

        let mut viewer = ComicViewer::new();
        viewer.load_file(path.clone());
        assert!(!viewer.thumbnail_cache_valid);
        assert!(
            !viewer.save_thumbnail_cache().unwrap(),
            "incompleta: no se guarda"
        );
        generate_all_thumbnails(&mut viewer);
        assert_eq!(viewer.thumbnail_jpegs.len(), 3);
        assert!(viewer.save_thumbnail_cache().unwrap());
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            modified_before,
            "se conserva la fecha de modificación"
        );

        let mut reopened = ComicViewer::new();
        reopened.load_file(path.clone());
        assert_eq!(reopened.pages.len(), 3, "babel-thb no cuenta como página");
        assert!(reopened.thumbnail_cache_valid);
        assert_eq!(reopened.page_thumbnails.len(), 3);
        assert!(
            reopened.take_thumbnail_request().is_none(),
            "nada que generar"
        );
        assert!(!reopened.save_thumbnail_cache().unwrap(), "no se reescribe");

        std::fs::remove_dir_all(&directory).unwrap();
    }
}
