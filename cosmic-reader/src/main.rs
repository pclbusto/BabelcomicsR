use cosmic::Element;
use cosmic::app::{Core, Task};
use cosmic::iced::{Limits, Size};
use std::env;
use std::path::PathBuf;

use cosmic::app::context_drawer::{self, ContextDrawer};
use cosmic::iced::widget::scrollable::RelativeOffset;
use cosmic::iced::window;
use cosmic::widget::{self, about::About, icon};
use cosmic_reader::options::{Fit, ReaderOptions};
use cosmic_reader::viewer::{ComicViewer, Panel, ThumbnailData, page_scroll_id, sidebar_scroll_id};

mod shortcuts;
mod shortcuts_panel;

/// Qué muestra el panel lateral derecho de COSMIC.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ContextPage {
    #[default]
    About,
    Shortcuts,
}

#[derive(Clone, Debug, Default)]
pub struct ReaderFlags {
    comic_path: Option<PathBuf>,
    progress_path: Option<PathBuf>,
    options: ReaderOptions,
}

pub fn main() -> cosmic::iced::Result {
    let settings = cosmic::app::Settings::default()
        .size(Size::new(800.0, 600.0))
        .size_limits(Limits::NONE.min_width(400.0).min_height(300.0));

    let mut flags = ReaderFlags::default();
    let mut args = env::args().skip(1);
    while let Some(argument) = args.next() {
        // Admite `--opcion valor` y `--opcion=valor`.
        let (name, inline_value) = match argument.split_once('=') {
            Some((name, value)) if name.starts_with("--") => {
                (name.to_string(), Some(value.to_string()))
            }
            _ => (argument.clone(), None),
        };
        if name == "--progress-file" || name == "--progress" {
            flags.progress_path = inline_value.or_else(|| args.next()).map(PathBuf::from);
        } else if matches!(
            name.as_str(),
            "--fit" | "--direction" | "--guided" | "--filter"
        ) {
            if let Some(value) = inline_value.or_else(|| args.next()) {
                flags.options.apply(&name, &value);
            }
        } else if flags.comic_path.is_none() {
            flags.comic_path = Some(PathBuf::from(argument));
        } else if flags.progress_path.is_none() {
            // También se acepta: cosmic-reader COMIC ARCHIVO_DE_PROGRESO
            flags.progress_path = Some(PathBuf::from(argument));
        }
    }

    cosmic::app::run::<ReaderApp>(settings, flags)
}

pub struct ReaderApp {
    core: Core,
    viewer: ComicViewer,
    about: About,
    context_page: ContextPage,
    shortcuts_query: String,
    shortcuts_filter: usize,
    fullscreen: bool,
}

#[derive(Clone, Debug)]
pub enum Message {
    NextPage,
    PreviousPage,
    /// Flechas del teclado; su sentido depende de la dirección de lectura.
    ArrowRight,
    ArrowLeft,
    ToggleMode,
    AddPanel(Panel),
    UpdatePanel(usize, Panel),
    SelectPanel(Option<usize>),
    DeleteSelectedPanel,
    SavePanels,
    EnableGuidedView(bool),
    TogglePageGrid,
    SelectPage(usize),
    ThumbnailBatchLoaded(Result<Vec<ThumbnailData>, String>),
    SetFit(Fit),
    ToggleFullscreen,
    ToggleSidebar,
    ToggleAbout,
    ToggleShortcuts,
    CloseContextDrawer,
    ShortcutsQueryChanged(String),
    ShortcutsFilterSelected(usize),
    OpenUrl(String),
    KeyPressed(cosmic::iced::keyboard::Event),
    /// La ventana se está cerrando: último momento para guardar la caché.
    Closing,
}

impl cosmic::Application for ReaderApp {
    type Executor = cosmic::iced::executor::Default;
    type Flags = ReaderFlags;
    type Message = Message;
    const APP_ID: &'static str = "com.babelcomics.cosmic-reader";

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, flags: Self::Flags) -> (Self, Task<Self::Message>) {
        let mut viewer = ComicViewer::new();
        viewer.options = flags.options;

        if let Some(path) = flags.comic_path {
            viewer.load_file(path);
            if let Some(progress_path) = flags.progress_path {
                viewer.configure_progress_file(progress_path);
            }
        }

        let about = About::default()
            .name("Lector de Babelcomics")
            .icon(icon::from_name(Self::APP_ID))
            .version(env!("CARGO_PKG_VERSION"))
            .author("GutenAIR")
            .license("MIT")
            .license_url("https://choosealicense.com/licenses/mit/")
            .developers([("Pedro César Luis Busto", "pclbusto@gmail.com")])
            .links([
                ("Repositorio", "https://github.com/pclbusto/BabelcomicsR"),
                ("Apoyo", "https://github.com/pclbusto/BabelcomicsR/issues"),
            ])
            .comments("Lector de cómics CBZ y CBR para Babelcomics, con lectura guiada por viñetas")
            .copyright("Copyright © 2026 Pedro César Luis Busto");

        let mut app = ReaderApp {
            core,
            viewer,
            about,
            context_page: ContextPage::default(),
            shortcuts_query: String::new(),
            shortcuts_filter: 0,
            fullscreen: false,
        };
        // El panel de miniaturas arranca visible: empieza a generarlas.
        let task = app.next_thumbnail_task();
        (app, task)
    }

    fn header_start(&self) -> Vec<Element<'_, Self::Message>> {
        vec![
            widget::tooltip(
                widget::button::icon(icon::from_name(if self.viewer.show_sidebar {
                    "navbar-open-symbolic"
                } else {
                    "navbar-closed-symbolic"
                }))
                .on_press(Message::ToggleSidebar),
                widget::text("Panel de miniaturas (F9)"),
                widget::tooltip::Position::Bottom,
            )
            .into(),
        ]
    }

    fn header_end(&self) -> Vec<Element<'_, Self::Message>> {
        vec![
            widget::tooltip(
                widget::button::icon(icon::from_name("input-keyboard-symbolic"))
                    .on_press(Message::ToggleShortcuts),
                widget::text("Atajos de teclado (Ctrl+F1)"),
                widget::tooltip::Position::Bottom,
            )
            .into(),
            widget::tooltip(
                widget::button::icon(icon::from_name("help-about-symbolic"))
                    .on_press(Message::ToggleAbout),
                widget::text("Acerca de (F1)"),
                widget::tooltip::Position::Bottom,
            )
            .into(),
        ]
    }

    fn context_drawer(&self) -> Option<ContextDrawer<'_, Self::Message>> {
        if !self.core.window.show_context {
            return None;
        }
        Some(match self.context_page {
            ContextPage::About => context_drawer::about(
                &self.about,
                |url| Message::OpenUrl(url.to_owned()),
                Message::CloseContextDrawer,
            ),
            ContextPage::Shortcuts => context_drawer::context_drawer(
                shortcuts_panel::view_content(&self.shortcuts_query, self.shortcuts_filter),
                Message::CloseContextDrawer,
            )
            .title("Atajos de teclado")
            .header(shortcuts_panel::view_header(
                &self.shortcuts_query,
                self.shortcuts_filter,
            )),
        })
    }

    fn on_escape(&mut self) -> Task<Self::Message> {
        if self.core.window.show_context {
            self.core.set_show_context(false);
            Task::none()
        } else if self.fullscreen {
            self.update(Message::ToggleFullscreen)
        } else {
            Task::none()
        }
    }

    fn on_close_requested(&self, _id: window::Id) -> Option<Self::Message> {
        Some(Message::Closing)
    }

    fn on_app_exit(&mut self) -> Option<Self::Message> {
        self.save_thumbnail_cache();
        None
    }

    fn update(&mut self, message: Self::Message) -> Task<Self::Message> {
        match message {
            Message::NextPage => return self.turn_page(true),
            Message::PreviousPage => return self.turn_page(false),
            Message::ArrowRight => return self.turn_page(!self.viewer.options.right_to_left),
            Message::ArrowLeft => return self.turn_page(self.viewer.options.right_to_left),
            Message::ToggleMode => self.viewer.toggle_mode(),
            Message::AddPanel(panel) => self.viewer.add_panel(panel),
            Message::UpdatePanel(i, p) => self.viewer.update_panel(i, p),
            Message::SelectPanel(i) => self.viewer.select_panel(i),
            Message::DeleteSelectedPanel => self.viewer.delete_selected_panel(),
            Message::SavePanels => self.viewer.save_panels(),
            Message::EnableGuidedView(enable) => self.viewer.set_guided_view(enable),
            Message::TogglePageGrid => {
                self.viewer.toggle_page_grid();
                return self.next_thumbnail_task();
            }
            Message::SelectPage(page_index) => {
                self.viewer.select_page(page_index);
                return Task::batch([scroll_page_to_start(), self.scroll_sidebar_to_current()]);
            }
            Message::SetFit(fit) => {
                self.viewer.options.fit = fit;
                return scroll_page_to_start();
            }
            Message::ToggleFullscreen => {
                self.fullscreen = !self.fullscreen;
                self.core.window.show_headerbar = !self.fullscreen;
                if let Some(id) = self.core.main_window_id() {
                    return window::set_mode(
                        id,
                        if self.fullscreen {
                            window::Mode::Fullscreen
                        } else {
                            window::Mode::Windowed
                        },
                    );
                }
            }
            Message::ToggleSidebar => {
                self.viewer.toggle_sidebar();
                return Task::batch([self.next_thumbnail_task(), self.scroll_sidebar_to_current()]);
            }
            Message::ToggleAbout => self.toggle_context_page(ContextPage::About),
            Message::ToggleShortcuts => self.toggle_context_page(ContextPage::Shortcuts),
            Message::CloseContextDrawer => self.core.set_show_context(false),
            Message::ShortcutsQueryChanged(query) => self.shortcuts_query = query,
            Message::ShortcutsFilterSelected(filter) => self.shortcuts_filter = filter,
            Message::OpenUrl(url) => {
                let _ = std::process::Command::new("xdg-open").arg(url).spawn();
            }
            Message::KeyPressed(key_event) => {
                if let cosmic::iced::keyboard::Event::KeyPressed {
                    key,
                    modified_key,
                    physical_key,
                    modifiers,
                    ..
                } = key_event
                    // `modified_key` resuelve la tecla según la distribución.
                    && let Some(message) =
                        shortcuts::message_for(modifiers, &modified_key, None).or_else(|| {
                            shortcuts::message_for(modifiers, &key, Some(&physical_key))
                        })
                {
                    return self.update(message);
                }
            }
            Message::Closing => self.save_thumbnail_cache(),
            Message::ThumbnailBatchLoaded(result) => {
                self.viewer.apply_thumbnail_batch(result);
                return self.next_thumbnail_task();
            }
        }
        Task::none()
    }

    fn subscription(&self) -> cosmic::iced::Subscription<Self::Message> {
        cosmic::iced::event::listen_with(|event, status, _id| match event {
            // Las teclas que consume un widget (p. ej. el buscador de atajos)
            // no se tratan como atajos.
            cosmic::iced::Event::Keyboard(
                key_event @ cosmic::iced::keyboard::Event::KeyPressed { .. },
            ) if status == cosmic::iced::event::Status::Ignored => {
                Some(Message::KeyPressed(key_event))
            }
            _ => None,
        })
    }

    fn view(&self) -> Element<'_, Self::Message> {
        self.viewer.view(
            Message::AddPanel,
            Message::UpdatePanel,
            Message::SelectPanel,
            Message::EnableGuidedView,
            Message::SelectPage,
            Message::TogglePageGrid,
        )
    }
}

fn scroll_page_to_start() -> Task<Message> {
    cosmic::iced::widget::scrollable::snap_to(page_scroll_id(), RelativeOffset::START.into())
}

impl ReaderApp {
    /// Muestra la página pedida en el panel derecho, o lo cierra si ya la mostraba.
    fn toggle_context_page(&mut self, page: ContextPage) {
        let showing = self.core.window.show_context && self.context_page == page;
        self.context_page = page;
        self.core.set_show_context(!showing);
    }

    /// Avanza o retrocede y, si cambió la página, vuelve al inicio del desplazamiento.
    fn turn_page(&mut self, forward: bool) -> Task<Message> {
        let before = self.viewer.current_page;
        if forward {
            self.viewer.next_page();
        } else {
            self.viewer.previous_page();
        }
        if self.viewer.current_page != before {
            Task::batch([scroll_page_to_start(), self.scroll_sidebar_to_current()])
        } else {
            Task::none()
        }
    }

    fn scroll_sidebar_to_current(&self) -> Task<Message> {
        if !self.viewer.show_sidebar {
            return Task::none();
        }
        cosmic::iced::widget::scrollable::snap_to(
            sidebar_scroll_id(),
            RelativeOffset {
                x: 0.0,
                y: self.viewer.sidebar_offset(),
            }
            .into(),
        )
    }

    /// Guarda `babel-thb` dentro del cómic si ya están todas las miniaturas.
    /// Es síncrono a propósito: la aplicación termina justo después.
    fn save_thumbnail_cache(&mut self) {
        match self.viewer.save_thumbnail_cache() {
            Ok(true) => eprintln!("cosmic-reader: caché de miniaturas guardada en el cómic"),
            Ok(false) => {}
            Err(error) => {
                eprintln!("cosmic-reader: no se pudo guardar la caché de miniaturas: {error}")
            }
        }
    }

    fn next_thumbnail_task(&mut self) -> Task<Message> {
        let Some(request) = self.viewer.take_thumbnail_request() else {
            return Task::none();
        };

        cosmic::task::future(async move {
            let result =
                tokio::task::spawn_blocking(move || ComicViewer::generate_thumbnail_batch(request))
                    .await
                    .map_err(|error| format!("Falló la generación de miniaturas: {error}"))
                    .and_then(|result| result);
            Message::ThumbnailBatchLoaded(result)
        })
    }
}
