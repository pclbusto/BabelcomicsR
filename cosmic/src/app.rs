use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use babelcomics_core::helpers::background_jobs::{
    self, BackgroundJobSnapshot, BackgroundJobStatus, JobKind,
};
use babelcomics_core::helpers::comicvine_client::{
    ComicVineClient, CvPublisherResult, CvVolumeResult,
};
use babelcomics_core::helpers::download_manager::{DownloadManager, DownloadStatus};
use babelcomics_core::helpers::organizer::{
    ComicOrganizationPlan, ComicOrganizer, OrganizationStatus,
};
use babelcomics_core::helpers::paths::{
    self, comic_thumbnail_path, read_comicbook_info_cover_bytes, volume_thumbnail_path,
};
use babelcomics_core::helpers::publisher_import_service;
use babelcomics_core::helpers::scan_service;
use babelcomics_core::helpers::suggestion_service::{
    SuggestionMethod, UnifiedSuggestion, suggest_best_matches,
};
use babelcomics_core::helpers::thumbnail::CardSize;
use babelcomics_core::models::{
    ComicFilter, ComicbookInfoView, ComicbookView, Publisher, ReaderDirection, ReaderFit,
    ReaderGuided, ReadingProgress, SetupDirectorio, Volume, VolumeView, parse_search_query,
};
use babelcomics_core::repositories::{
    ComicbookInfoRepository, ComicbookRepository, PublisherRepository, ReadingProgressRepository,
    SetupRepository, VolumeRepository, VolumeSortOrder,
};
use cosmic::app::{Task, context_drawer};
use cosmic::iced::advanced::text::{Ellipsize, EllipsizeHeightLimit};
use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::{Alignment, Event, Length, Subscription, event, keyboard};
use cosmic::prelude::*;
use cosmic::widget::{self, icon, nav_bar};
use sqlx::SqlitePool;

const PAGE_LIMIT: i64 = 120;
const COMIC_CARD_WIDTH: f32 = 194.0;
const COMIC_COVER_WIDTH: f32 = 170.0;
const COMIC_COVER_HEIGHT: f32 = 238.0;
const COMIC_GRID_GAP: f32 = 12.0;
const COMIC_GRID_SCROLLBAR_RESERVE: f32 = 20.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Page {
    Comics,
    Volumes,
    Publishers,
    ComicVine,
    Downloads,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum ComicVineSearchMode {
    #[default]
    Volumes,
    Publishers,
}

enum EitherComicVineResults {
    Volumes(Vec<CvVolumeResult>),
    Publishers(Vec<CvPublisherResult>),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum CatalogFilter {
    #[default]
    All,
    Catalogued,
    Uncatalogued,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum VolumeUpdateFilter {
    #[default]
    All,
    Updated,
    NotUpdated,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum PreferencesPage {
    #[default]
    Menu,
    General,
    ComicVine,
    ScanFolders,
    NormalizeCatalog,
    Reader,
    Advanced,
}

#[derive(Clone, Debug)]
pub(crate) struct VolumeListItem {
    volume: VolumeView,
    has_api_updates: bool,
}

/// Posición de la grilla de cómics, para conservarla cuando cambia el diseño.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ComicGridViewport {
    offset_y: f32,
    content_height: f32,
    columns: usize,
    /// Si el panel de detalle estaba visible.
    context_shown: bool,
}

impl ComicGridViewport {
    fn row_height(&self, comics: usize) -> f32 {
        let rows = comics.div_ceil(self.columns.max(1)).max(1);
        self.content_height / rows as f32
    }
}

const COMIC_GRID_SCROLL_ID: &str = "comic-grid";

#[derive(Clone, Debug, Default)]
pub(crate) struct LibraryStats {
    comics: i64,
    uncatalogued: i64,
    errors: i64,
    volumes: i64,
    publishers: i64,
    directories: Vec<SetupDirectorio>,
    thumbnail_size: i64,
    thumbnails_directory: Option<String>,
    normalized_catalog_directory: Option<String>,
    items_per_page: i64,
    workers: i64,
    reader_filter: i64,
    reader_path: Option<String>,
    clip_on_startup: bool,
    clip_on_download: bool,
    reader_fit: ReaderFit,
    reader_direction: ReaderDirection,
    reader_guided: ReaderGuided,
    scan_extensions: BTreeSet<String>,
    api_key: Option<String>,
    api_url: String,
    clip_total: i64,
    clip_local: i64,
    clip_indexed: i64,
    clip_pending: i64,
    /// Portadas sin embedding que ya tienen archivo local (se pueden procesar ya).
    clip_pending_local: i64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum VolumeDetailTab {
    #[default]
    Information,
    Issues,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum IssueOwnershipFilter {
    #[default]
    All,
    Owned,
    Missing,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum IssueDetailTab {
    #[default]
    Information,
    Covers,
    PhysicalComics,
}

#[derive(Clone, Debug)]
pub(crate) struct VolumeDetailData {
    volume: Volume,
    publisher: Option<Publisher>,
    issues: Vec<ComicbookInfoView>,
    clip_total: i64,
    clip_local: i64,
    clip_indexed: i64,
    clip_pending: i64,
    issue_covers: HashMap<i64, Vec<ImageHandle>>,
}

#[derive(Clone, Debug)]
pub(crate) struct IssueDetailData {
    issues: Vec<ComicbookInfoView>,
    physical_comics: Vec<(i64, ComicbookView)>,
    issue_covers: HashMap<i64, Vec<ImageHandle>>,
}

type SelectedVolumesQueueResult = Result<(Vec<(i64, i64)>, usize, usize), String>;
type ImageHandle = cosmic::iced::widget::image::Handle;
type CatalogCandidatesResult = Result<(Vec<UnifiedSuggestion>, HashMap<i64, ImageHandle>), String>;

#[derive(Clone, Debug)]
struct CoverRequest {
    id: i64,
    ruta_local: Option<String>,
    url_original: Option<String>,
    volume_name: String,
    volume_id: i64,
}

fn is_executable_file(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

fn comicvine_volume_page_url(volume: &Volume) -> Option<String> {
    let stored = volume.url.trim();
    if (stored.starts_with("https://") || stored.starts_with("http://"))
        && !stored.contains("/api/")
    {
        return Some(stored.to_string());
    }
    volume
        .id_comicvine
        .map(|id| format!("https://comicvine.gamespot.com/volume/4050-{id}/"))
}

fn comicvine_issue_page_url(issue: &ComicbookInfoView) -> Option<String> {
    if let Some(stored) = issue.info.url_api_detalle.as_deref().map(str::trim)
        && (stored.starts_with("https://") || stored.starts_with("http://"))
        && !stored.contains("/api/")
    {
        return Some(stored.to_string());
    }
    issue
        .info
        .id_comicvine
        .map(|id| format!("https://comicvine.gamespot.com/issue/4000-{id}/"))
}

#[derive(Clone, Debug)]
pub(crate) enum Drawer {
    About,
    Preferences,
    SearchFilters,
    Comic(Box<ComicbookView>),
    Comics(Vec<ComicbookView>),
    Volumes(Vec<VolumeView>),
    Publisher(Publisher),
}

#[derive(Clone, Debug)]
pub(crate) enum DeleteTarget {
    Comic {
        id: i64,
        name: String,
        path: String,
    },
    Comics {
        items: Vec<(i64, String)>,
        label: String,
    },
    Volume {
        id: i64,
        name: String,
    },
    Publisher {
        id: i64,
        name: String,
    },
}

impl DeleteTarget {
    fn kind(&self) -> &'static str {
        match self {
            Self::Comic { .. } => "cómic",
            Self::Comics { .. } => "cómics seleccionados",
            Self::Volume { .. } => "volumen",
            Self::Publisher { .. } => "editorial",
        }
    }

    fn name(&self) -> &str {
        match self {
            Self::Comic { name, .. } | Self::Volume { name, .. } | Self::Publisher { name, .. } => {
                name
            }
            Self::Comics { label, .. } => label,
        }
    }

    fn has_physical_files(&self) -> bool {
        matches!(self, Self::Comic { .. } | Self::Comics { .. })
    }
}

#[derive(Clone, Debug)]
pub enum Message {
    SearchChanged(String),
    ActivateSearch(String),
    CloseSearch,
    Search,
    Refresh,
    ComicsLoaded(Result<(Vec<ComicbookView>, i64), String>),
    VolumesLoaded(Result<Vec<VolumeListItem>, String>),
    VolumeDetailLoaded(Box<Result<VolumeDetailData, String>>),
    CloseVolumeDetail,
    UpdateVolume,
    VolumeClicked(i64),
    UpdateSelectedVolumes,
    SelectedVolumesQueued(SelectedVolumesQueueResult),
    SelectVolumeDetailTab(VolumeDetailTab),
    VolumeIssueSearchChanged(String),
    SetIssueOwnershipFilter(IssueOwnershipFilter),
    IssueClicked(i64),
    OpenSelectedIssueDetail,
    IssueDetailLoaded(Box<Result<IssueDetailData, String>>),
    SelectIssueDetailTab(IssueDetailTab),
    CloseIssueDetail,
    UnlinkPhysicalComic(i64),
    PhysicalComicUnlinked(i64, Result<(), String>),
    ReadComic(i64, String),
    OpenExternalUrl(String),
    ReaderFinished(i64, Result<ReadingProgress, String>),
    PendingReadingProgressImported(Result<(usize, Vec<String>), String>),
    OpenCatalog(Box<ComicbookView>),
    OpenCatalogBatch(Vec<ComicbookView>),
    CatalogLoaded(CatalogCandidatesResult),
    SelectCatalogCandidate(usize),
    ApplyCatalog,
    OpenManualCatalog,
    CloseManualCatalog,
    ManualCatalogQueryChanged(String),
    ManualCatalogVolumesLoaded(Result<Vec<VolumeView>, String>),
    SelectManualCatalogVolume(i64),
    ManualCatalogIssuesLoaded(CatalogCandidatesResult),
    SkipCatalog,
    CatalogApplied(Result<(), String>),
    CloseCatalog,
    GenerateClipEmbeddings(Option<i64>),
    /// Genera los embeddings faltantes sin que lo pida el usuario.
    AutoGenerateClipEmbeddings,
    /// Revisa si terminó alguna descarga de volumen.
    CheckDownloadsForClip,
    SetClipOnStartup(bool),
    SetClipOnDownload(bool),
    ClipGenerationFinished(Option<i64>, Result<(u32, usize), String>),
    PublishersLoaded(Result<Vec<Publisher>, String>),
    StatsLoaded(Box<Result<LibraryStats, String>>),
    DirectoryInputChanged(String),
    ChooseDirectories,
    DirectoriesChosen(Result<Vec<String>, String>),
    AddDirectory,
    RemoveDirectory(i64),
    DirectoriesSaved(Result<(), String>),
    StartScan,
    ScanFinished(Result<(String, bool), String>),
    SelectPreferencesPage(PreferencesPage),
    SetThumbnailSize(i64),
    SetReaderFilter(i64),
    SetReaderFit(ReaderFit),
    SetReaderDirection(ReaderDirection),
    SetReaderGuided(ReaderGuided),
    ChooseReaderExecutable,
    ReaderExecutableChosen(Result<Option<String>, String>),
    ResetReaderExecutable,
    ChooseNormalizedCatalogDirectory,
    NormalizedCatalogDirectoryChosen(Result<Option<String>, String>),
    ChooseThumbnailsDirectory,
    ThumbnailsDirectoryChosen(Result<Option<String>, String>),
    PreferenceDirectorySaved(Result<String, String>),
    PrepareOrganization,
    OrganizationPlanReady(Result<Vec<ComicOrganizationPlan>, String>),
    CancelOrganization,
    ConfirmOrganization,
    OrganizationFinished(Result<(usize, usize, usize), String>),
    SetScanExtension(String, bool),
    ApiKeyChanged(String),
    ApiUrlChanged(String),
    SaveApiSettings,
    ApiSettingsSaved(Result<(), String>),
    TestComicVineConnection,
    ComicVineConnectionTested(Result<(), String>),
    ComicVineQueryChanged(String),
    SetComicVineSearchMode(ComicVineSearchMode),
    SearchComicVine,
    ComicVineSearchFinished(Result<Vec<CvVolumeResult>, String>),
    ComicVinePublisherSearchFinished(Result<Vec<CvPublisherResult>, String>),
    ToggleComicVineSelection(i64),
    ToggleComicVinePublisherSelection(i64),
    ToggleDownloadCovers(bool),
    QueueComicVineDownloads,
    ImportComicVinePublishers,
    ComicVinePublishersImported(Result<(Vec<String>, usize, usize), String>),
    SetCatalogFilter(CatalogFilter),
    SetVolumeUpdateFilter(VolumeUpdateFilter),
    SetVolumeSortOrder(VolumeSortOrder),
    FilterPreferencesLoaded(Result<(CatalogFilter, VolumeUpdateFilter), String>),
    FilterPreferencesSaved(Result<(), String>),
    VolumeSortOrderLoaded(Result<VolumeSortOrder, String>),
    VolumeSortOrderSaved(Result<(), String>),
    PreferenceSaved(Result<(), String>),
    RequestDelete(Box<DeleteTarget>),
    CancelDelete,
    ConfirmDelete,
    ToggleDeletePhysical(bool),
    DeleteFinished(Result<Option<String>, String>),
    ModifiersChanged {
        control: bool,
        shift: bool,
    },
    ComicClicked(i64),
    ComicGridScrolled(ComicGridViewport),
    /// Doble clic o "Más opciones": selecciona el cómic y abre su detalle.
    OpenComicDetail(i64),
    ShowSelectionDetails,
    OpenDrawer(Box<Drawer>),
    CloseDrawer,
    Tick,
}

pub struct Babelcomics {
    core: cosmic::Core,
    pool: SqlitePool,
    nav: nav_bar::Model,
    search: String,
    search_active: bool,
    search_input_id: widget::Id,
    comics: Vec<ComicbookView>,
    comics_total: i64,
    volumes: Vec<VolumeListItem>,
    volume_detail: Option<VolumeDetailData>,
    volume_detail_loading: bool,
    volume_detail_error: Option<String>,
    volume_detail_tab: VolumeDetailTab,
    volume_updates: BTreeMap<i64, i64>,
    volume_issue_search: String,
    volume_issue_filter: IssueOwnershipFilter,
    selected_issues: BTreeSet<i64>,
    issue_selection_anchor: Option<i64>,
    issue_detail: Option<IssueDetailData>,
    issue_detail_loading: bool,
    issue_detail_error: Option<String>,
    issue_detail_tab: IssueDetailTab,
    reader_error: Option<String>,
    active_readers: BTreeSet<i64>,
    catalog_comic: Option<ComicbookView>,
    catalog_queue: Vec<ComicbookView>,
    catalog_queue_index: usize,
    catalog_completed: usize,
    catalog_skipped: usize,
    catalog_candidates: Vec<UnifiedSuggestion>,
    catalog_candidate_covers: HashMap<i64, ImageHandle>,
    catalog_selected: usize,
    catalog_loading: bool,
    catalog_applying: bool,
    catalog_error: Option<String>,
    catalog_manual: bool,
    manual_catalog_query: String,
    manual_catalog_volumes: Vec<VolumeView>,
    manual_catalog_volume: Option<VolumeView>,
    manual_catalog_loading: bool,
    manual_catalog_error: Option<String>,
    clip_generating: bool,
    /// La generación en curso la lanzó la app sola (al iniciar o tras una descarga).
    clip_run_automatic: bool,
    /// Se pidió otra generación automática mientras había una en curso.
    clip_rerun_requested: bool,
    /// Ya se evaluó la generación al iniciar.
    clip_startup_checked: bool,
    /// Descargas terminadas cuyas portadas ya se enviaron a CLIP.
    clip_seen_downloads: HashSet<String>,
    publishers: Vec<Publisher>,
    jobs: Vec<BackgroundJobSnapshot>,
    stats: LibraryStats,
    directory_input: String,
    directory_error: Option<String>,
    directory_saving: bool,
    scanning: bool,
    scan_status: Option<String>,
    preferences_page: PreferencesPage,
    preference_saving: bool,
    preference_error: Option<String>,
    organization_plan: Vec<ComicOrganizationPlan>,
    organization_preparing: bool,
    organizing: bool,
    api_key_input: String,
    api_url_input: String,
    api_saving: bool,
    api_testing: bool,
    api_status: Option<String>,
    api_initialized: bool,
    catalog_filter: CatalogFilter,
    volume_update_filter: VolumeUpdateFilter,
    volume_sort_order: VolumeSortOrder,
    comicvine_query: String,
    comicvine_search_mode: ComicVineSearchMode,
    comicvine_results: Vec<CvVolumeResult>,
    comicvine_publisher_results: Vec<CvPublisherResult>,
    comicvine_selected: BTreeSet<i64>,
    comicvine_publishers_selected: BTreeSet<i64>,
    comicvine_searching: bool,
    comicvine_importing: bool,
    comicvine_has_searched: bool,
    comicvine_error: Option<String>,
    comicvine_download_covers: bool,
    database_path: String,
    pending_delete: Option<DeleteTarget>,
    deleting: bool,
    delete_error: Option<String>,
    delete_physical_files: bool,
    control_pressed: bool,
    shift_pressed: bool,
    selected_comics: BTreeSet<i64>,
    /// Última posición conocida de la grilla de cómics.
    comic_grid_viewport: ComicGridViewport,
    selection_anchor: Option<i64>,
    selected_volumes: BTreeSet<i64>,
    volume_selection_anchor: Option<i64>,
    drawer: Option<Drawer>,
    loading: bool,
    error: Option<String>,
    notice: Option<String>,
}

impl cosmic::Application for Babelcomics {
    type Executor = cosmic::executor::Default;
    type Flags = SqlitePool;
    type Message = Message;

    const APP_ID: &'static str = "com.github.babelcomics.Cosmic";

    fn core(&self) -> &cosmic::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut cosmic::Core {
        &mut self.core
    }

    fn init(core: cosmic::Core, pool: Self::Flags) -> (Self, Task<Self::Message>) {
        let mut nav = nav_bar::Model::default();
        nav.insert()
            .text("Cómics")
            .icon(icon::from_name("x-office-document-symbolic"))
            .data(Page::Comics)
            .activate();
        nav.insert()
            .text("Series")
            .icon(icon::from_name("view-grid-symbolic"))
            .data(Page::Volumes);
        nav.insert()
            .text("Editoriales")
            .icon(icon::from_name("avatar-default-symbolic"))
            .data(Page::Publishers);
        nav.insert()
            .text("ComicVine")
            .icon(icon::from_name("system-search-symbolic"))
            .data(Page::ComicVine);
        nav.insert()
            .text("Actividad")
            .icon(icon::from_name("folder-download-symbolic"))
            .data(Page::Downloads);

        let mut app = Self {
            core,
            pool,
            nav,
            search: String::new(),
            search_active: false,
            search_input_id: widget::Id::unique(),
            comics: Vec::new(),
            comics_total: 0,
            volumes: Vec::new(),
            volume_detail: None,
            volume_detail_loading: false,
            volume_detail_error: None,
            volume_detail_tab: VolumeDetailTab::Information,
            volume_updates: BTreeMap::new(),
            volume_issue_search: String::new(),
            volume_issue_filter: IssueOwnershipFilter::All,
            selected_issues: BTreeSet::new(),
            issue_selection_anchor: None,
            issue_detail: None,
            issue_detail_loading: false,
            issue_detail_error: None,
            issue_detail_tab: IssueDetailTab::Information,
            reader_error: None,
            active_readers: BTreeSet::new(),
            catalog_comic: None,
            catalog_queue: Vec::new(),
            catalog_queue_index: 0,
            catalog_completed: 0,
            catalog_skipped: 0,
            catalog_candidates: Vec::new(),
            catalog_candidate_covers: HashMap::new(),
            catalog_selected: 0,
            catalog_loading: false,
            catalog_applying: false,
            catalog_error: None,
            catalog_manual: false,
            manual_catalog_query: String::new(),
            manual_catalog_volumes: Vec::new(),
            manual_catalog_volume: None,
            manual_catalog_loading: false,
            manual_catalog_error: None,
            clip_generating: false,
            clip_run_automatic: false,
            clip_rerun_requested: false,
            clip_startup_checked: false,
            clip_seen_downloads: HashSet::new(),
            publishers: Vec::new(),
            jobs: background_jobs::snapshot(),
            stats: LibraryStats::default(),
            directory_input: String::new(),
            directory_error: None,
            directory_saving: false,
            scanning: false,
            scan_status: None,
            preferences_page: PreferencesPage::Menu,
            preference_saving: false,
            preference_error: None,
            organization_plan: Vec::new(),
            organization_preparing: false,
            organizing: false,
            api_key_input: String::new(),
            api_url_input: "https://comicvine.gamespot.com/api/".to_string(),
            api_saving: false,
            api_testing: false,
            api_status: None,
            api_initialized: false,
            catalog_filter: CatalogFilter::All,
            volume_update_filter: VolumeUpdateFilter::All,
            volume_sort_order: VolumeSortOrder::default(),
            comicvine_query: String::new(),
            comicvine_search_mode: ComicVineSearchMode::Volumes,
            comicvine_results: Vec::new(),
            comicvine_publisher_results: Vec::new(),
            comicvine_selected: BTreeSet::new(),
            comicvine_publishers_selected: BTreeSet::new(),
            comicvine_searching: false,
            comicvine_importing: false,
            comicvine_has_searched: false,
            comicvine_error: None,
            comicvine_download_covers: true,
            database_path: babelcomics_core::bootstrap::get_db_path(),
            pending_delete: None,
            deleting: false,
            delete_error: None,
            delete_physical_files: false,
            control_pressed: false,
            shift_pressed: false,
            selected_comics: BTreeSet::new(),
            comic_grid_viewport: ComicGridViewport::default(),
            selection_anchor: None,
            selected_volumes: BTreeSet::new(),
            volume_selection_anchor: None,
            drawer: None,
            loading: true,
            error: None,
            notice: None,
        };

        let tasks = Task::batch([
            app.load_comics(),
            app.load_volumes(),
            app.load_publishers(),
            app.load_stats(),
            app.load_filter_preferences(),
            app.load_volume_sort_order(),
            app.import_pending_reading_progress(),
            app.update_title(),
        ]);
        (app, tasks)
    }

    fn nav_model(&self) -> Option<&nav_bar::Model> {
        Some(&self.nav)
    }

    fn on_nav_select(&mut self, id: nav_bar::Id) -> Task<Self::Message> {
        self.nav.activate(id);
        self.volume_detail = None;
        self.volume_detail_loading = false;
        self.volume_detail_error = None;
        self.clear_catalog();
        self.search.clear();
        self.search_active = false;
        self.selected_comics.clear();
        self.selection_anchor = None;
        self.selected_volumes.clear();
        self.volume_selection_anchor = None;
        self.selected_issues.clear();
        self.issue_selection_anchor = None;
        self.close_issue_detail();
        self.error = None;
        self.update_title()
    }

    fn header_end(&self) -> Vec<Element<'_, Self::Message>> {
        let mut elements = Vec::with_capacity(4);
        if self.search_active {
            let placeholder = match self
                .nav
                .active_data::<Page>()
                .copied()
                .unwrap_or(Page::Comics)
            {
                Page::Comics => "Buscar cómics…",
                Page::Volumes => "Buscar series…",
                Page::Publishers => "Buscar editoriales…",
                Page::ComicVine => "Buscar en ComicVine…",
                Page::Downloads => "Buscar…",
            };
            elements.push(
                widget::search_input(placeholder, &self.search)
                    .id(self.search_input_id.clone())
                    .on_input(Message::SearchChanged)
                    .on_submit(|_| Message::Search)
                    .on_clear(Message::CloseSearch)
                    .width(360)
                    .into(),
            );
        }
        if self
            .nav
            .active_data::<Page>()
            .is_some_and(|page| matches!(page, Page::Comics | Page::Volumes))
        {
            let catalog_label = match self.catalog_filter {
                CatalogFilter::All => "Todos",
                CatalogFilter::Catalogued => "Catalogados",
                CatalogFilter::Uncatalogued => "Sin catalogar",
            };
            let filter_label = if self.nav.active_data::<Page>() == Some(&Page::Volumes) {
                let update_label = match self.volume_update_filter {
                    VolumeUpdateFilter::All => "todas",
                    VolumeUpdateFilter::Updated => "actualizadas",
                    VolumeUpdateFilter::NotUpdated => "sin actualizar",
                };
                format!("{catalog_label} · {update_label}")
            } else {
                catalog_label.to_string()
            };
            elements.push(
                widget::button::standard(filter_label)
                    .leading_icon(icon::from_name("view-more-symbolic"))
                    .on_press(Message::OpenDrawer(Box::new(Drawer::SearchFilters)))
                    .into(),
            );
        }
        elements.extend([
            widget::tooltip(
                widget::button::icon(icon::from_name("view-refresh-symbolic"))
                    .on_press(Message::Refresh),
                widget::text("Actualizar"),
                widget::tooltip::Position::Bottom,
            )
            .into(),
            widget::tooltip(
                widget::button::icon(icon::from_name("preferences-system-symbolic"))
                    .on_press(Message::OpenDrawer(Box::new(Drawer::Preferences))),
                widget::text("Preferencias"),
                widget::tooltip::Position::Bottom,
            )
            .into(),
            widget::tooltip(
                widget::button::icon(icon::from_name("help-about-symbolic"))
                    .on_press(Message::OpenDrawer(Box::new(Drawer::About))),
                widget::text("Acerca de"),
                widget::tooltip::Position::Bottom,
            )
            .into(),
        ]);
        elements
    }

    fn context_drawer(&self) -> Option<context_drawer::ContextDrawer<'_, Self::Message>> {
        let drawer = self.drawer.as_ref()?;
        let (title, content) = match drawer {
            Drawer::About => ("Babelcomics", self.about_view()),
            Drawer::Preferences => ("Preferencias", self.preferences_view()),
            Drawer::SearchFilters => ("Parámetros de búsqueda", self.search_filters_view()),
            Drawer::Comic(item) => ("Detalle del cómic", self.comic_detail(item)),
            Drawer::Comics(items) => ("Selección de cómics", self.comics_selection_detail(items)),
            Drawer::Volumes(items) => (
                "Selección de volúmenes",
                self.volumes_selection_detail(items),
            ),
            Drawer::Publisher(item) => ("Editorial", self.publisher_detail(item)),
        };
        Some(context_drawer::context_drawer(content, Message::CloseDrawer).title(title))
    }

    fn dialog(&self) -> Option<Element<'_, Self::Message>> {
        let target = self.pending_delete.as_ref()?;
        let body = self.delete_error.clone().unwrap_or_else(|| {
            if self.delete_physical_files && target.has_physical_files() {
                format!(
                    "Se eliminará «{}» de la base de datos y del disco. El borrado físico es permanente y no se puede deshacer.",
                    target.name()
                )
            } else {
                format!(
                    "Se eliminará «{}» solamente de la base de datos. Los archivos del disco se conservarán.",
                    target.name()
                )
            }
        });
        let confirm = widget::button::destructive(if self.deleting {
            "Eliminando…"
        } else {
            "Eliminar"
        })
        .on_press_maybe((!self.deleting).then_some(Message::ConfirmDelete));
        let cancel = widget::button::standard("Cancelar")
            .on_press_maybe((!self.deleting).then_some(Message::CancelDelete));

        let mut dialog = widget::dialog()
            .title(format!("¿Eliminar {}?", target.kind()))
            .body(body)
            .icon(icon::from_name("dialog-warning-symbolic").size(32).icon())
            .primary_action(confirm)
            .secondary_action(cancel);
        if target.has_physical_files() {
            dialog = dialog.control(
                widget::checkbox(self.delete_physical_files)
                    .label("Eliminar también los archivos físicos")
                    .on_toggle(Message::ToggleDeletePhysical),
            );
        }
        Some(dialog.into())
    }

    fn view(&self) -> Element<'_, Self::Message> {
        if self.catalog_comic.is_some() {
            return self.catalog_view();
        }
        if self.issue_detail_loading
            || self.issue_detail.is_some()
            || self.issue_detail_error.is_some()
        {
            return self.issue_detail_view();
        }
        if self.volume_detail_loading
            || self.volume_detail.is_some()
            || self.volume_detail_error.is_some()
        {
            return self.volume_detail_view();
        }
        let body = match self
            .nav
            .active_data::<Page>()
            .copied()
            .unwrap_or(Page::Comics)
        {
            Page::Comics => self.comics_view(),
            Page::Volumes => self.volumes_view(),
            Page::Publishers => self.publishers_view(),
            Page::ComicVine => self.comicvine_view(),
            Page::Downloads => self.downloads_view(),
        };

        widget::container(body)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        Subscription::batch([
            cosmic::iced::time::every(Duration::from_secs(1)).map(|_| Message::Tick),
            cosmic::iced::time::every(Duration::from_secs(2))
                .map(|_| Message::CheckDownloadsForClip),
            event::listen_with(|event, status, _| match event {
                Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                    Some(Message::ModifiersChanged {
                        control: modifiers.control(),
                        shift: modifiers.shift(),
                    })
                }
                Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. })
                    if modifiers.control()
                        && matches!(&key, keyboard::Key::Character(character) if character.as_str() == " ") =>
                {
                    Some(Message::ShowSelectionDetails)
                }
                Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. })
                    if status == event::Status::Ignored
                        && !modifiers.control()
                        && !modifiers.alt()
                        && !modifiers.logo()
                        && matches!(&key, keyboard::Key::Character(character) if !character.is_empty()) =>
                {
                    let keyboard::Key::Character(character) = key else {
                        return None;
                    };
                    Some(Message::ActivateSearch(character.to_string()))
                }
                _ => None,
            }),
        ])
    }

    fn update(&mut self, message: Self::Message) -> Task<Self::Message> {
        match message {
            Message::SearchChanged(value) => {
                self.search = value;
                self.loading = true;
                return self.load_active_page();
            }
            Message::ActivateSearch(text) => {
                if self.is_searchable_page() {
                    self.search_active = true;
                    self.search.push_str(&text);
                    self.loading = true;
                    let focus = widget::text_input::focus(self.search_input_id.clone())
                        .map(cosmic::Action::App);
                    return Task::batch([self.load_active_page(), focus]);
                }
            }
            Message::CloseSearch => {
                self.search_active = false;
                self.search.clear();
                self.loading = true;
                return self.load_active_page();
            }
            Message::Search => {
                self.loading = true;
                self.error = None;
                return self.load_active_page();
            }
            Message::Refresh => {
                self.loading = true;
                self.error = None;
                self.notice = None;
                if let Some(comic_id) = self.catalog_comic.as_ref().map(|comic| comic.id_comicbook)
                {
                    self.catalog_candidates.clear();
                    self.catalog_candidate_covers.clear();
                    self.catalog_selected = 0;
                    self.catalog_loading = true;
                    self.catalog_error = None;
                    self.loading = false;
                    return self.load_catalog_candidates(comic_id);
                }
                if let Some(volume_id) = self
                    .volume_detail
                    .as_ref()
                    .map(|detail| detail.volume.id_volume)
                {
                    self.volume_detail = None;
                    self.volume_detail_loading = true;
                    self.loading = false;
                    return self.load_volume_detail(volume_id);
                }
                return Task::batch([
                    self.load_comics(),
                    self.load_volumes(),
                    self.load_publishers(),
                    self.load_stats(),
                ]);
            }
            Message::ComicsLoaded(result) => match result {
                Ok((items, total)) => {
                    self.comics = items;
                    self.comics_total = total;
                    self.loading = false;
                }
                Err(error) => self.fail(error),
            },
            Message::VolumesLoaded(result) => match result {
                Ok(items) => {
                    self.volumes = items;
                    self.loading = false;
                }
                Err(error) => self.fail(error),
            },
            Message::VolumeDetailLoaded(result) => {
                self.volume_detail_loading = false;
                match *result {
                    Ok(detail) => self.volume_detail = Some(detail),
                    Err(error) => self.volume_detail_error = Some(error),
                }
            }
            Message::CloseVolumeDetail => {
                self.volume_detail = None;
                self.volume_detail_loading = false;
                self.volume_detail_error = None;
                self.volume_issue_search.clear();
                self.volume_issue_filter = IssueOwnershipFilter::All;
                self.selected_issues.clear();
                self.issue_selection_anchor = None;
                self.close_issue_detail();
            }
            Message::UpdateVolume => {
                let Some(detail) = self.volume_detail.as_ref() else {
                    return Task::none();
                };
                let Some(cv_id) = detail.volume.id_comicvine else {
                    self.notice = Some(
                        "Este volumen no tiene un ID de ComicVine y no se puede actualizar."
                            .to_string(),
                    );
                    return Task::none();
                };
                let manager = DownloadManager::get_instance(self.pool.clone());
                let queued = manager.add_download(
                    cv_id,
                    &detail.volume.nombre,
                    detail.volume.cantidad_numeros,
                    true,
                );
                self.volume_updates.insert(cv_id, detail.volume.id_volume);
                self.notice = Some(if queued {
                    "Actualizando metadatos, números y portadas desde ComicVine…".to_string()
                } else {
                    "El volumen ya se está actualizando; puedes ver el progreso en Actividad."
                        .to_string()
                });
                self.jobs = background_jobs::snapshot();
            }
            Message::VolumeClicked(id) => return self.select_volume(id),
            Message::UpdateSelectedVolumes => {
                if self.selected_volumes.is_empty() {
                    return Task::none();
                }
                return self.queue_selected_volume_updates();
            }
            Message::SelectedVolumesQueued(result) => match result {
                Ok((tracked, added, without_comicvine_id)) => {
                    for (cv_id, volume_id) in &tracked {
                        self.volume_updates.insert(*cv_id, *volume_id);
                    }
                    self.notice = Some(format!(
                        "{} volumen(es) agregados a la cola{}.",
                        added,
                        if without_comicvine_id > 0 {
                            format!(" · {without_comicvine_id} sin ID de ComicVine")
                        } else {
                            String::new()
                        }
                    ));
                    self.selected_volumes.clear();
                    self.volume_selection_anchor = None;
                    self.drawer = None;
                    self.core.window.show_context = false;
                    self.jobs = background_jobs::snapshot();
                }
                Err(error) => self.error = Some(error),
            },
            Message::SelectVolumeDetailTab(tab) => self.volume_detail_tab = tab,
            Message::VolumeIssueSearchChanged(query) => self.volume_issue_search = query,
            Message::SetIssueOwnershipFilter(filter) => self.volume_issue_filter = filter,
            Message::IssueClicked(id) => self.select_issue(id),
            Message::OpenSelectedIssueDetail => {
                return self.open_selected_issue_detail();
            }
            Message::IssueDetailLoaded(result) => {
                if !self.issue_detail_loading {
                    return Task::none();
                }
                self.issue_detail_loading = false;
                match *result {
                    Ok(detail) => self.issue_detail = Some(detail),
                    Err(error) => self.issue_detail_error = Some(error),
                }
            }
            Message::SelectIssueDetailTab(tab) => self.issue_detail_tab = tab,
            Message::CloseIssueDetail => self.close_issue_detail(),
            Message::UnlinkPhysicalComic(comic_id) => {
                self.issue_detail_error = None;
                return self.unlink_physical_comic(comic_id);
            }
            Message::PhysicalComicUnlinked(comic_id, result) => match result {
                Ok(()) => {
                    if let Some(detail) = self.issue_detail.as_mut() {
                        detail
                            .physical_comics
                            .retain(|(_, comic)| comic.id_comicbook != comic_id);
                        for issue in &mut detail.issues {
                            issue.physical_count = detail
                                .physical_comics
                                .iter()
                                .filter(|(issue_id, _)| *issue_id == issue.info.id_comicbook_info)
                                .count() as i64;
                        }
                    }
                    self.notice = Some("El cómic quedó sin clasificar.".to_string());
                    let mut tasks =
                        vec![self.load_comics(), self.load_volumes(), self.load_stats()];
                    if let Some(volume_id) = self
                        .volume_detail
                        .as_ref()
                        .map(|detail| detail.volume.id_volume)
                    {
                        tasks.push(self.load_volume_detail(volume_id));
                    }
                    return Task::batch(tasks);
                }
                Err(error) => self.issue_detail_error = Some(error),
            },
            Message::ReadComic(comic_id, path) => {
                self.reader_error = None;
                if !self.active_readers.insert(comic_id) {
                    self.notice = Some("Ese cómic ya está abierto en el lector.".to_string());
                    return Task::none();
                }
                self.notice =
                    Some("Lector abierto; el progreso se guardará al cerrarlo.".to_string());
                return self.launch_reader(comic_id, path);
            }
            Message::OpenExternalUrl(url) => {
                if !url.starts_with("https://") && !url.starts_with("http://") {
                    self.error = Some("El enlace externo no es una URL HTTP válida.".to_string());
                } else if let Err(error) = open::that_detached(&url) {
                    self.error = Some(format!("No se pudo abrir el enlace: {error}"));
                }
            }
            Message::ReaderFinished(comic_id, result) => {
                self.active_readers.remove(&comic_id);
                match result {
                    Ok(progress) => {
                        let visited = progress.visited_page_indexes.len();
                        let total = progress.total_pages;
                        let percentage = if total > 0 {
                            visited as f64 / total as f64 * 100.0
                        } else {
                            0.0
                        };
                        self.notice = Some(format!(
                            "Progreso de lectura guardado: {visited} de {total} páginas ({percentage:.0}%)."
                        ));
                        if let Some(detail) = self.issue_detail.as_mut()
                            && let Some((_, comic)) = detail
                                .physical_comics
                                .iter_mut()
                                .find(|(_, comic)| comic.id_comicbook == comic_id)
                        {
                            comic.reading_total_pages = total as i64;
                            comic.reading_visited_pages = visited as i64;
                            comic.reading_last_page_index =
                                progress.last_page_index.map(|index| index as i64);
                            comic.reading_completed_at = progress.completed_at;
                        }
                        return self.load_comics();
                    }
                    Err(error) => self.reader_error = Some(error),
                }
            }
            Message::PendingReadingProgressImported(result) => match result {
                Ok((imported, errors)) => {
                    if imported > 0 {
                        self.notice = Some(format!(
                            "Se recuperó el progreso pendiente de {imported} cómic(s)."
                        ));
                    }
                    if !errors.is_empty() {
                        self.reader_error = Some(format!(
                            "No se pudo recuperar todo el progreso pendiente: {}",
                            errors.join(" · ")
                        ));
                    }
                    if imported > 0 {
                        return self.load_comics();
                    }
                }
                Err(error) => {
                    self.reader_error = Some(format!(
                        "No se pudo revisar el progreso de lectura pendiente: {error}"
                    ));
                }
            },
            Message::OpenCatalog(comic) => {
                return self.start_catalog_queue(vec![*comic]);
            }
            Message::OpenCatalogBatch(comics) => {
                if comics.is_empty() {
                    return Task::none();
                }
                return self.start_catalog_queue(comics);
            }
            Message::CatalogLoaded(result) => {
                self.catalog_loading = false;
                match result {
                    Ok((candidates, covers)) => {
                        self.catalog_candidates = candidates;
                        self.catalog_candidate_covers = covers;
                    }
                    Err(error) => self.catalog_error = Some(error),
                }
            }
            Message::SelectCatalogCandidate(index) => {
                if index < self.catalog_candidates.len() && !self.catalog_applying {
                    self.catalog_selected = index;
                }
            }
            Message::ApplyCatalog => {
                if self.catalog_applying {
                    return Task::none();
                }
                let Some(comic) = self.catalog_comic.as_ref() else {
                    return Task::none();
                };
                let Some(candidate) = self.catalog_candidates.get(self.catalog_selected) else {
                    return Task::none();
                };
                self.catalog_applying = true;
                self.catalog_error = None;
                if self.catalog_manual && self.manual_catalog_volume.is_some() {
                    return self
                        .apply_manual_catalog(comic.id_comicbook, candidate.id_comicbook_info);
                }
                return self.apply_catalog_candidate(
                    comic.id_comicbook,
                    candidate.clone(),
                    self.catalog_selected,
                );
            }
            Message::OpenManualCatalog => {
                self.catalog_manual = true;
                self.manual_catalog_volume = None;
                self.manual_catalog_error = None;
                self.manual_catalog_loading = true;
                return self.load_manual_catalog_volumes();
            }
            Message::CloseManualCatalog => {
                if self.manual_catalog_volume.is_some() {
                    self.manual_catalog_volume = None;
                    self.catalog_candidates.clear();
                    self.catalog_candidate_covers.clear();
                    self.catalog_selected = 0;
                    self.manual_catalog_error = None;
                } else {
                    self.catalog_manual = false;
                    self.manual_catalog_error = None;
                    if self.catalog_candidates.is_empty()
                        && let Some(comic_id) =
                            self.catalog_comic.as_ref().map(|comic| comic.id_comicbook)
                    {
                        self.catalog_loading = true;
                        return self.load_catalog_candidates(comic_id);
                    }
                }
            }
            Message::ManualCatalogQueryChanged(query) => {
                self.manual_catalog_query = query;
                self.manual_catalog_loading = true;
                self.manual_catalog_error = None;
                return self.load_manual_catalog_volumes();
            }
            Message::ManualCatalogVolumesLoaded(result) => {
                self.manual_catalog_loading = false;
                match result {
                    Ok(volumes) => self.manual_catalog_volumes = volumes,
                    Err(error) => self.manual_catalog_error = Some(error),
                }
            }
            Message::SelectManualCatalogVolume(volume_id) => {
                let Some(volume) = self
                    .manual_catalog_volumes
                    .iter()
                    .find(|volume| volume.id_volume == volume_id)
                    .cloned()
                else {
                    return Task::none();
                };
                self.manual_catalog_volume = Some(volume);
                self.catalog_candidates.clear();
                self.catalog_candidate_covers.clear();
                self.catalog_selected = 0;
                self.manual_catalog_loading = true;
                self.manual_catalog_error = None;
                return self.load_manual_catalog_issues(volume_id);
            }
            Message::ManualCatalogIssuesLoaded(result) => {
                self.manual_catalog_loading = false;
                match result {
                    Ok((candidates, covers)) => {
                        self.catalog_candidates = candidates;
                        self.catalog_candidate_covers = covers;
                        self.catalog_selected = 0;
                    }
                    Err(error) => self.manual_catalog_error = Some(error),
                }
            }
            Message::CatalogApplied(result) => {
                self.catalog_applying = false;
                match result {
                    Ok(()) => {
                        self.catalog_completed += 1;
                        if let Some(task) = self.advance_catalog_queue() {
                            return task;
                        }
                        let completed = self.catalog_completed;
                        let skipped = self.catalog_skipped;
                        self.notice = Some(format!(
                            "Catalogación finalizada: {completed} catalogado(s) · {skipped} omitido(s)."
                        ));
                        self.clear_catalog();
                        return Task::batch([
                            self.load_comics(),
                            self.load_volumes(),
                            self.load_stats(),
                        ]);
                    }
                    Err(error) => {
                        if self.catalog_manual && self.manual_catalog_volume.is_some() {
                            self.manual_catalog_error = Some(error);
                        } else {
                            self.catalog_error = Some(error);
                        }
                    }
                }
            }
            Message::SkipCatalog => {
                if !self.catalog_applying {
                    self.catalog_skipped += 1;
                    if let Some(task) = self.advance_catalog_queue() {
                        return task;
                    }
                    self.notice = Some(format!(
                        "Catalogación finalizada: {} catalogado(s) · {} omitido(s).",
                        self.catalog_completed, self.catalog_skipped
                    ));
                    self.clear_catalog();
                    return Task::batch([
                        self.load_comics(),
                        self.load_volumes(),
                        self.load_stats(),
                    ]);
                }
            }
            Message::CloseCatalog => self.clear_catalog(),
            Message::AutoGenerateClipEmbeddings => {
                if self.clip_generating {
                    self.clip_rerun_requested = true;
                    return Task::none();
                }
                self.clip_generating = true;
                self.clip_run_automatic = true;
                return self.generate_clip_embeddings(None);
            }
            Message::CheckDownloadsForClip => {
                let manager = DownloadManager::get_instance(self.pool.clone());
                let state = manager.get_state();
                let state = state.lock().unwrap();
                let mut finished = false;
                for (key, download) in state.iter() {
                    if matches!(download.status, DownloadStatus::Completed) {
                        finished |= self.clip_seen_downloads.insert(key.clone());
                    } else {
                        // Vuelve a contar si el mismo volumen se baja otra vez.
                        self.clip_seen_downloads.remove(key);
                    }
                }
                drop(state);
                if finished && self.stats.clip_on_download {
                    return self.update(Message::AutoGenerateClipEmbeddings);
                }
            }
            Message::SetClipOnStartup(enabled) => {
                return self.save_setup_preference(move |repository| {
                    Box::pin(async move { repository.set_clip_al_arranque(enabled).await })
                });
            }
            Message::SetClipOnDownload(enabled) => {
                return self.save_setup_preference(move |repository| {
                    Box::pin(async move { repository.set_clip_al_descargar(enabled).await })
                });
            }
            Message::GenerateClipEmbeddings(volume_id) => {
                if self.clip_generating {
                    return Task::none();
                }
                self.clip_generating = true;
                self.clip_run_automatic = false;
                self.preference_error = None;
                self.notice = Some(
                    "Preparando el índice visual CLIP; puedes seguir el progreso en Actividad."
                        .to_string(),
                );
                return self.generate_clip_embeddings(volume_id);
            }
            Message::ClipGenerationFinished(volume_id, result) => {
                self.clip_generating = false;
                let automatic = std::mem::take(&mut self.clip_run_automatic);
                match result {
                    // Una pasada automática sin nada que hacer no merece aviso.
                    Ok((0, 0)) if automatic => {}
                    Ok((generated, errors)) => {
                        self.notice = Some(if generated == 0 && errors == 0 {
                            "No había embeddings CLIP pendientes para generar.".to_string()
                        } else {
                            format!(
                                "Índice CLIP actualizado: {generated} embeddings generados · {errors} errores."
                            )
                        });
                    }
                    Err(error) => {
                        self.notice = Some(format!("No se pudo generar el índice CLIP: {error}"));
                        self.preference_error = Some(error);
                    }
                }

                let mut tasks = vec![self.load_stats()];
                if std::mem::take(&mut self.clip_rerun_requested) {
                    tasks.push(self.update(Message::AutoGenerateClipEmbeddings));
                }
                if let Some(volume_id) = volume_id {
                    self.volume_detail = None;
                    self.volume_detail_loading = true;
                    tasks.push(self.load_volume_detail(volume_id));
                }
                return Task::batch(tasks);
            }
            Message::PublishersLoaded(result) => match result {
                Ok(items) => {
                    self.publishers = items;
                    self.loading = false;
                }
                Err(error) => self.fail(error),
            },
            Message::StatsLoaded(result) => match *result {
                Ok(stats) => {
                    if !self.api_initialized {
                        self.api_key_input = stats.api_key.clone().unwrap_or_default();
                        self.api_url_input = stats.api_url.clone();
                        self.api_initialized = true;
                    }
                    let generate_on_startup = !self.clip_startup_checked
                        && stats.clip_on_startup
                        && stats.clip_pending_local > 0;
                    self.clip_startup_checked = true;
                    self.stats = stats;
                    if generate_on_startup {
                        return self.update(Message::AutoGenerateClipEmbeddings);
                    }
                }
                Err(error) => self.error = Some(error),
            },
            Message::DirectoryInputChanged(value) => {
                self.directory_input = value;
                self.directory_error = None;
            }
            Message::ChooseDirectories => {
                return cosmic::task::future(async {
                    use cosmic::dialog::file_chooser;

                    let dialog =
                        file_chooser::open::Dialog::new().title("Elegir carpetas de cómics");
                    let result = match dialog.open_folders().await {
                        Ok(response) => response
                            .urls()
                            .iter()
                            .map(|url| {
                                url.to_file_path()
                                    .map(|path| path.to_string_lossy().into_owned())
                                    .map_err(|_| {
                                        "La carpeta elegida no es una ruta local".to_string()
                                    })
                            })
                            .collect(),
                        Err(file_chooser::Error::Cancelled) => Ok(Vec::new()),
                        Err(error) => Err(format!("No se pudo abrir el selector: {error}")),
                    };
                    Message::DirectoriesChosen(result)
                });
            }
            Message::DirectoriesChosen(result) => match result {
                Ok(paths) if paths.is_empty() => {}
                Ok(paths) => return self.save_directories(paths),
                Err(error) => self.directory_error = Some(error),
            },
            Message::AddDirectory => {
                let path = self.directory_input.trim().to_owned();
                if path.is_empty() {
                    self.directory_error = Some("Escribe o elige una carpeta".to_string());
                } else {
                    return self.save_directories(vec![path]);
                }
            }
            Message::RemoveDirectory(id) => {
                self.directory_saving = true;
                self.directory_error = None;
                let pool = self.pool.clone();
                return Task::perform(
                    async move {
                        SetupRepository::new(&pool)
                            .remove_directorio(id)
                            .await
                            .map_err(|error| error.to_string())
                    },
                    |result| cosmic::Action::App(Message::DirectoriesSaved(result)),
                );
            }
            Message::DirectoriesSaved(result) => {
                self.directory_saving = false;
                match result {
                    Ok(()) => {
                        self.directory_input.clear();
                        self.directory_error = None;
                        return self.load_stats();
                    }
                    Err(error) => self.directory_error = Some(error),
                }
            }
            Message::StartScan => {
                if self.scanning {
                    return Task::none();
                }
                self.scanning = true;
                self.scan_status = Some("Preparando escaneo…".to_string());
                scan_service::STOP_THREADS.store(false, Ordering::Relaxed);
                return self.run_scan();
            }
            Message::ScanFinished(result) => {
                self.scanning = false;
                self.scan_status = Some(match result {
                    Ok((summary, _)) => summary,
                    Err(error) => error,
                });
                return Task::batch([
                    self.load_comics(),
                    self.load_volumes(),
                    self.load_publishers(),
                    self.load_stats(),
                ]);
            }
            Message::SelectPreferencesPage(page) => {
                self.preferences_page = page;
                self.preference_error = None;
            }
            Message::SetThumbnailSize(size) => {
                self.preference_saving = true;
                self.preference_error = None;
                let pool = self.pool.clone();
                return Task::perform(
                    async move {
                        SetupRepository::new(&pool)
                            .set_card_size(size)
                            .await
                            .map_err(|error| error.to_string())
                    },
                    |result| cosmic::Action::App(Message::PreferenceSaved(result)),
                );
            }
            Message::SetReaderFilter(filter) => {
                return self.save_setup_preference(move |repository| {
                    Box::pin(async move { repository.set_reader_filter(filter).await })
                });
            }
            Message::SetReaderFit(fit) => {
                return self.save_setup_preference(move |repository| {
                    Box::pin(async move { repository.set_reader_fit(fit).await })
                });
            }
            Message::SetReaderDirection(direction) => {
                return self.save_setup_preference(move |repository| {
                    Box::pin(async move { repository.set_reader_direction(direction).await })
                });
            }
            Message::SetReaderGuided(guided) => {
                return self.save_setup_preference(move |repository| {
                    Box::pin(async move { repository.set_reader_guided(guided).await })
                });
            }
            Message::ResetReaderExecutable => {
                return self.save_setup_preference(|repository| {
                    Box::pin(async move { repository.set_reader_path(None).await })
                });
            }
            Message::ChooseReaderExecutable => {
                if self.preference_saving {
                    return Task::none();
                }
                return cosmic::task::future(async {
                    use cosmic::dialog::file_chooser;

                    let dialog =
                        file_chooser::open::Dialog::new().title("Elegir el ejecutable del lector");
                    let result = match dialog.open_file().await {
                        Ok(response) => response
                            .url()
                            .to_file_path()
                            .map(|path| Some(path.to_string_lossy().into_owned()))
                            .map_err(|_| "El archivo elegido no es una ruta local".to_string()),
                        Err(file_chooser::Error::Cancelled) => Ok(None),
                        Err(error) => Err(format!("No se pudo abrir el selector: {error}")),
                    };
                    Message::ReaderExecutableChosen(result)
                });
            }
            Message::ReaderExecutableChosen(result) => match result {
                Ok(Some(path)) => {
                    if !is_executable_file(std::path::Path::new(&path)) {
                        self.preference_error =
                            Some(format!("El archivo no es ejecutable: {path}"));
                        return Task::none();
                    }
                    return self.save_setup_preference(move |repository| {
                        Box::pin(async move { repository.set_reader_path(Some(&path)).await })
                    });
                }
                Ok(None) => {}
                Err(error) => self.preference_error = Some(error),
            },
            Message::ChooseNormalizedCatalogDirectory => {
                if self.preference_saving {
                    return Task::none();
                }
                return cosmic::task::future(async {
                    use cosmic::dialog::file_chooser;

                    let dialog = file_chooser::open::Dialog::new()
                        .title("Elegir carpeta para el catálogo normalizado");
                    let result = match dialog.open_folders().await {
                        Ok(response) => response
                            .urls()
                            .first()
                            .map(|url| {
                                url.to_file_path()
                                    .map(|path| path.to_string_lossy().into_owned())
                                    .map_err(|_| {
                                        "La carpeta elegida no es una ruta local".to_string()
                                    })
                            })
                            .transpose(),
                        Err(file_chooser::Error::Cancelled) => Ok(None),
                        Err(error) => Err(format!("No se pudo abrir el selector: {error}")),
                    };
                    Message::NormalizedCatalogDirectoryChosen(result)
                });
            }
            Message::NormalizedCatalogDirectoryChosen(result) => match result {
                Ok(Some(path)) => {
                    self.preference_saving = true;
                    self.preference_error = None;
                    return self.save_normalized_catalog_directory(path);
                }
                Ok(None) => {}
                Err(error) => self.preference_error = Some(error),
            },
            Message::ChooseThumbnailsDirectory => {
                if self.preference_saving {
                    return Task::none();
                }
                return cosmic::task::future(async {
                    use cosmic::dialog::file_chooser;

                    let dialog = file_chooser::open::Dialog::new()
                        .title("Elegir carpeta para las miniaturas");
                    let result = match dialog.open_folders().await {
                        Ok(response) => response
                            .urls()
                            .first()
                            .map(|url| {
                                url.to_file_path()
                                    .map(|path| path.to_string_lossy().into_owned())
                                    .map_err(|_| {
                                        "La carpeta elegida no es una ruta local".to_string()
                                    })
                            })
                            .transpose(),
                        Err(file_chooser::Error::Cancelled) => Ok(None),
                        Err(error) => Err(format!("No se pudo abrir el selector: {error}")),
                    };
                    Message::ThumbnailsDirectoryChosen(result)
                });
            }
            Message::ThumbnailsDirectoryChosen(result) => match result {
                Ok(Some(path)) => {
                    self.preference_saving = true;
                    self.preference_error = None;
                    return self.save_thumbnails_directory(path);
                }
                Ok(None) => {}
                Err(error) => self.preference_error = Some(error),
            },
            Message::PreferenceDirectorySaved(result) => {
                self.preference_saving = false;
                match result {
                    Ok(message) => {
                        self.organization_plan.clear();
                        self.notice = Some(message);
                        return self.load_stats();
                    }
                    Err(error) => self.preference_error = Some(error),
                }
            }
            Message::PrepareOrganization => {
                if self.organization_preparing || self.organizing {
                    return Task::none();
                }
                if self.stats.normalized_catalog_directory.is_none() {
                    self.preference_error =
                        Some("Configura primero la carpeta del catálogo normalizado.".to_string());
                    return Task::none();
                }
                self.organization_preparing = true;
                self.organization_plan.clear();
                self.preference_error = None;
                return self.prepare_organization();
            }
            Message::OrganizationPlanReady(result) => {
                self.organization_preparing = false;
                match result {
                    Ok(plan) => self.organization_plan = plan,
                    Err(error) => self.preference_error = Some(error),
                }
            }
            Message::CancelOrganization => {
                if !self.organizing {
                    self.organization_plan.clear();
                }
            }
            Message::ConfirmOrganization => {
                if self.organizing || self.organization_plan.is_empty() {
                    return Task::none();
                }
                self.organizing = true;
                self.preference_error = None;
                self.notice = Some(
                    "Organizando los cómics; puedes seguir el progreso en Actividad…".to_string(),
                );
                return self.execute_organization();
            }
            Message::OrganizationFinished(result) => {
                self.organizing = false;
                self.organization_plan.clear();
                match result {
                    Ok((moved, failed, skipped)) => {
                        self.notice = Some(format!(
                            "Organización terminada: {moved} movidos · {skipped} omitidos · {failed} errores."
                        ));
                        return Task::batch([
                            self.load_comics(),
                            self.load_volumes(),
                            self.load_stats(),
                        ]);
                    }
                    Err(error) => {
                        self.preference_error = Some(error.clone());
                        self.notice = Some(format!("No se pudo organizar el catálogo: {error}"));
                    }
                }
            }
            Message::SetScanExtension(extension, enabled) => {
                self.preference_saving = true;
                self.preference_error = None;
                let pool = self.pool.clone();
                return Task::perform(
                    async move {
                        SetupRepository::new(&pool)
                            .set_scan_extension(&extension, enabled)
                            .await
                            .map_err(|error| error.to_string())
                    },
                    |result| cosmic::Action::App(Message::PreferenceSaved(result)),
                );
            }
            Message::ApiKeyChanged(value) => {
                self.api_key_input = value;
                self.api_status = None;
            }
            Message::ApiUrlChanged(value) => {
                self.api_url_input = value;
                self.api_status = None;
            }
            Message::SaveApiSettings => {
                self.api_saving = true;
                self.api_status = Some("Guardando configuración…".to_string());
                return self.save_api_settings();
            }
            Message::ApiSettingsSaved(result) => {
                self.api_saving = false;
                match result {
                    Ok(()) => {
                        self.api_status = Some("Configuración guardada.".to_string());
                        return self.load_stats();
                    }
                    Err(error) => self.api_status = Some(format!("Error: {error}")),
                }
            }
            Message::TestComicVineConnection => {
                if self.api_key_input.trim().is_empty() {
                    self.api_status = Some("Introduce una API key.".to_string());
                } else {
                    self.api_testing = true;
                    self.api_status = Some("Probando conexión…".to_string());
                    return self.test_comicvine_connection();
                }
            }
            Message::ComicVineConnectionTested(result) => {
                self.api_testing = false;
                self.api_status = Some(match result {
                    Ok(()) => "Conexión establecida correctamente.".to_string(),
                    Err(error) => format!("No se pudo conectar: {error}"),
                });
            }
            Message::ComicVineQueryChanged(query) => {
                self.comicvine_query = query;
                self.comicvine_error = None;
            }
            Message::SetComicVineSearchMode(mode) => {
                if self.comicvine_search_mode != mode
                    && !self.comicvine_searching
                    && !self.comicvine_importing
                {
                    self.comicvine_search_mode = mode;
                    self.comicvine_results.clear();
                    self.comicvine_publisher_results.clear();
                    self.comicvine_selected.clear();
                    self.comicvine_publishers_selected.clear();
                    self.comicvine_has_searched = false;
                    self.comicvine_error = None;
                }
            }
            Message::SearchComicVine => {
                if self.comicvine_query.trim().is_empty()
                    || self.comicvine_searching
                    || self.comicvine_importing
                {
                    return Task::none();
                }
                self.comicvine_searching = true;
                self.comicvine_error = None;
                self.comicvine_results.clear();
                self.comicvine_publisher_results.clear();
                self.comicvine_selected.clear();
                self.comicvine_publishers_selected.clear();
                return self.search_comicvine();
            }
            Message::ComicVineSearchFinished(result) => {
                self.comicvine_searching = false;
                self.comicvine_has_searched = true;
                match result {
                    Ok(results) => self.comicvine_results = results,
                    Err(error) => self.comicvine_error = Some(error),
                }
            }
            Message::ComicVinePublisherSearchFinished(result) => {
                self.comicvine_searching = false;
                self.comicvine_has_searched = true;
                match result {
                    Ok(results) => self.comicvine_publisher_results = results,
                    Err(error) => self.comicvine_error = Some(error),
                }
            }
            Message::ToggleComicVineSelection(id) => {
                if !self.comicvine_selected.insert(id) {
                    self.comicvine_selected.remove(&id);
                }
            }
            Message::ToggleComicVinePublisherSelection(id) => {
                if !self.comicvine_publishers_selected.insert(id) {
                    self.comicvine_publishers_selected.remove(&id);
                }
            }
            Message::ToggleDownloadCovers(enabled) => {
                self.comicvine_download_covers = enabled;
            }
            Message::QueueComicVineDownloads => {
                let selected = self
                    .comicvine_results
                    .iter()
                    .filter(|volume| self.comicvine_selected.contains(&volume.cv_id))
                    .cloned()
                    .collect::<Vec<_>>();
                if selected.is_empty() {
                    return Task::none();
                }
                let manager = DownloadManager::get_instance(self.pool.clone());
                let mut queued = 0usize;
                for volume in &selected {
                    if manager.add_download(
                        volume.cv_id,
                        &volume.name,
                        volume.count_of_issues,
                        self.comicvine_download_covers,
                    ) {
                        queued += 1;
                    }
                }
                self.notice = Some(if queued == 0 {
                    "Los volúmenes seleccionados ya estaban en la cola.".to_string()
                } else {
                    format!("{queued} volumen(es) agregados a la cola de descargas.")
                });
                self.comicvine_selected.clear();
                self.jobs = background_jobs::snapshot();
            }
            Message::ImportComicVinePublishers => {
                if self.comicvine_publishers_selected.is_empty() || self.comicvine_importing {
                    return Task::none();
                }
                self.comicvine_importing = true;
                self.comicvine_error = None;
                self.notice =
                    Some("Importando editorial y obteniendo todos sus volúmenes…".to_string());
                return self.import_comicvine_publishers();
            }
            Message::ComicVinePublishersImported(result) => {
                self.comicvine_importing = false;
                match result {
                    Ok((names, inserted, skipped)) => {
                        self.comicvine_publishers_selected.clear();
                        self.notice = Some(format!(
                            "{}: {inserted} volúmenes importados · {skipped} ya existentes.",
                            names.join(", ")
                        ));
                        return Task::batch([
                            self.load_volumes(),
                            self.load_publishers(),
                            self.load_stats(),
                        ]);
                    }
                    Err(error) => {
                        self.comicvine_error = Some(format!("No se pudo importar: {error}"));
                        self.notice = None;
                    }
                }
            }
            Message::SetCatalogFilter(filter) => {
                self.catalog_filter = filter;
                self.loading = true;
                return Task::batch([
                    self.load_comics(),
                    self.load_volumes(),
                    self.save_filter_preferences(),
                ]);
            }
            Message::SetVolumeUpdateFilter(filter) => {
                self.volume_update_filter = filter;
                self.loading = true;
                return Task::batch([self.load_volumes(), self.save_filter_preferences()]);
            }
            Message::SetVolumeSortOrder(sort_order) => {
                self.volume_sort_order = sort_order;
                self.loading = true;
                return Task::batch([self.load_volumes(), self.save_volume_sort_order()]);
            }
            Message::FilterPreferencesLoaded(result) => match result {
                Ok((catalog_filter, volume_update_filter)) => {
                    self.catalog_filter = catalog_filter;
                    self.volume_update_filter = volume_update_filter;
                    self.loading = true;
                    return Task::batch([self.load_comics(), self.load_volumes()]);
                }
                Err(error) => self.preference_error = Some(error),
            },
            Message::FilterPreferencesSaved(result) => {
                if let Err(error) = result {
                    self.preference_error = Some(error);
                }
            }
            Message::VolumeSortOrderLoaded(result) => match result {
                Ok(sort_order) => {
                    self.volume_sort_order = sort_order;
                    self.loading = true;
                    return self.load_volumes();
                }
                Err(error) => self.preference_error = Some(error),
            },
            Message::VolumeSortOrderSaved(result) => {
                if let Err(error) = result {
                    self.preference_error = Some(error);
                }
            }
            Message::PreferenceSaved(result) => {
                self.preference_saving = false;
                match result {
                    Ok(()) => return self.load_stats(),
                    Err(error) => self.preference_error = Some(error),
                }
            }
            Message::RequestDelete(target) => {
                self.pending_delete = Some(*target);
                self.delete_error = None;
                self.delete_physical_files = false;
            }
            Message::CancelDelete => {
                if !self.deleting {
                    self.pending_delete = None;
                    self.delete_error = None;
                    self.delete_physical_files = false;
                }
            }
            Message::ToggleDeletePhysical(enabled) => {
                if !self.deleting {
                    self.delete_physical_files = enabled;
                    self.delete_error = None;
                }
            }
            Message::ConfirmDelete => {
                let Some(target) = self.pending_delete.clone() else {
                    return Task::none();
                };
                self.deleting = true;
                self.delete_error = None;
                return self.delete_item(target, self.delete_physical_files);
            }
            Message::DeleteFinished(result) => {
                self.deleting = false;
                match result {
                    Ok(warning) => {
                        self.pending_delete = None;
                        self.drawer = None;
                        self.volume_detail = None;
                        self.volume_detail_error = None;
                        self.selected_comics.clear();
                        self.selection_anchor = None;
                        self.selected_volumes.clear();
                        self.volume_selection_anchor = None;
                        self.delete_physical_files = false;
                        self.notice = warning;
                        self.core.window.show_context = false;
                        return Task::batch([
                            self.load_comics(),
                            self.load_volumes(),
                            self.load_publishers(),
                            self.load_stats(),
                        ]);
                    }
                    Err(error) => self.delete_error = Some(format!("No se pudo eliminar: {error}")),
                }
            }
            Message::ModifiersChanged { control, shift } => {
                let modifier_started =
                    (control && !self.control_pressed) || (shift && !self.shift_pressed);
                self.control_pressed = control;
                self.shift_pressed = shift;
                if modifier_started
                    && matches!(
                        self.drawer,
                        Some(Drawer::Comic(_) | Drawer::Comics(_) | Drawer::Volumes(_))
                    )
                {
                    self.drawer = None;
                    self.core.window.show_context = false;
                }
            }
            Message::ComicClicked(id) => self.select_comic(id),
            Message::ComicGridScrolled(mut viewport) => {
                viewport.context_shown = self.core.window.show_context;
                let previous = self.comic_grid_viewport;
                let layout_changed = previous.columns > 0
                    && (previous.columns != viewport.columns
                        || previous.context_shown != viewport.context_shown);
                if !layout_changed || self.comics.is_empty() {
                    self.comic_grid_viewport = viewport;
                    return Task::none();
                }
                // Abrir o cerrar el panel de detalle reconstruye la grilla (pierde
                // el desplazamiento) y puede cambiar las columnas: se vuelve a la
                // fila del primer cómic que estaba visible.
                let old_row_height = previous.row_height(self.comics.len());
                let new_row_height = viewport.row_height(self.comics.len());
                if old_row_height <= 0.0 || new_row_height <= 0.0 {
                    self.comic_grid_viewport = viewport;
                    return Task::none();
                }
                let old_row = (previous.offset_y / old_row_height).floor();
                let within_row = previous.offset_y - old_row * old_row_height;
                let first_visible = old_row as usize * previous.columns;
                let new_row = (first_visible / viewport.columns.max(1)) as f32;
                let target = (new_row * new_row_height + within_row).max(0.0);
                viewport.offset_y = target;
                self.comic_grid_viewport = viewport;
                return cosmic::iced::widget::scrollable::scroll_to(
                    widget::Id::new(COMIC_GRID_SCROLL_ID),
                    cosmic::iced::widget::scrollable::AbsoluteOffset {
                        x: None,
                        y: Some(target),
                    },
                );
            }
            Message::OpenComicDetail(id) => {
                self.selected_comics.clear();
                self.selected_comics.insert(id);
                self.selection_anchor = Some(id);
                self.show_selected_comics();
            }
            Message::ShowSelectionDetails => {
                if self.volume_detail.is_some()
                    && self.volume_detail_tab == VolumeDetailTab::Issues
                    && !self.selected_issues.is_empty()
                    && self.issue_detail.is_none()
                    && !self.issue_detail_loading
                {
                    return self.open_selected_issue_detail();
                }
                match self.nav.active_data::<Page>() {
                    Some(Page::Comics) if !self.selected_comics.is_empty() => {
                        self.show_selected_comics();
                    }
                    Some(Page::Volumes) if !self.selected_volumes.is_empty() => {
                        self.show_selected_volumes();
                    }
                    _ => {}
                }
            }
            Message::OpenDrawer(drawer) => {
                self.selected_comics.clear();
                self.selection_anchor = None;
                self.selected_volumes.clear();
                self.volume_selection_anchor = None;
                self.reader_error = None;
                let drawer = *drawer;
                if matches!(drawer, Drawer::Preferences) {
                    self.preferences_page = PreferencesPage::Menu;
                }
                self.drawer = Some(drawer);
                self.core.window.show_context = true;
            }
            Message::CloseDrawer => {
                self.core.window.show_context = false;
                self.drawer = None;
                self.selected_comics.clear();
                self.selection_anchor = None;
                self.selected_volumes.clear();
                self.volume_selection_anchor = None;
                self.reader_error = None;
            }
            Message::Tick => {
                self.jobs = background_jobs::snapshot();
                if !self.volume_updates.is_empty() {
                    let manager = DownloadManager::get_instance(self.pool.clone());
                    let state = manager.get_state();
                    let state = state.lock().unwrap();
                    let mut completed = Vec::new();
                    let mut errors = Vec::new();
                    for (&cv_id, &volume_id) in &self.volume_updates {
                        match state
                            .get(&cv_id.to_string())
                            .map(|download| &download.status)
                        {
                            Some(DownloadStatus::Completed) => {
                                completed.push((cv_id, volume_id));
                            }
                            Some(DownloadStatus::Error(error)) => {
                                errors.push((cv_id, error.clone()));
                            }
                            _ => {}
                        }
                    }
                    drop(state);

                    for (cv_id, _) in &completed {
                        self.volume_updates.remove(cv_id);
                    }
                    for (cv_id, _) in &errors {
                        self.volume_updates.remove(cv_id);
                    }
                    if !completed.is_empty() || !errors.is_empty() {
                        self.notice = Some(if errors.is_empty() {
                            format!(
                                "{} volumen(es) actualizados correctamente.",
                                completed.len()
                            )
                        } else {
                            format!(
                                "{} volumen(es) actualizados · {} con error.",
                                completed.len(),
                                errors.len()
                            )
                        });
                        let mut tasks = vec![
                            self.load_volumes(),
                            self.load_publishers(),
                            self.load_stats(),
                        ];
                        if let Some(volume_id) = self.volume_detail.as_ref().and_then(|detail| {
                            completed
                                .iter()
                                .find(|(cv_id, _)| detail.volume.id_comicvine == Some(*cv_id))
                                .map(|(_, volume_id)| *volume_id)
                        }) {
                            self.volume_detail = None;
                            self.volume_detail_loading = true;
                            tasks.push(self.load_volume_detail(volume_id));
                        }
                        return Task::batch(tasks);
                    }
                }
            }
        }
        Task::none()
    }

    fn on_app_exit(&mut self) -> Option<Self::Message> {
        scan_service::STOP_THREADS.store(true, Ordering::Relaxed);
        None
    }

    fn on_escape(&mut self) -> Task<Self::Message> {
        if self.pending_delete.is_some() && !self.deleting {
            self.pending_delete = None;
            self.delete_error = None;
            self.delete_physical_files = false;
        } else if self.catalog_comic.is_some() {
            self.clear_catalog();
        } else if self.issue_detail.is_some()
            || self.issue_detail_loading
            || self.issue_detail_error.is_some()
        {
            self.close_issue_detail();
        } else if self.selected_comics.len() > 1 {
            self.selected_comics.clear();
            self.selection_anchor = None;
            self.drawer = None;
            self.core.window.show_context = false;
        } else if self.selected_volumes.len() > 1 {
            self.selected_volumes.clear();
            self.volume_selection_anchor = None;
            self.drawer = None;
            self.core.window.show_context = false;
        } else if self.drawer.is_some() {
            self.drawer = None;
            self.core.window.show_context = false;
        } else if self.volume_detail.is_some() || self.volume_detail_error.is_some() {
            self.volume_detail = None;
            self.volume_detail_error = None;
        } else if self.search_active {
            self.search_active = false;
            self.search.clear();
            self.loading = true;
            return self.load_active_page();
        }
        Task::none()
    }
}

impl Babelcomics {
    fn is_searchable_page(&self) -> bool {
        self.catalog_comic.is_none()
            && self.volume_detail.is_none()
            && !self.volume_detail_loading
            && self.volume_detail_error.is_none()
            && matches!(
                self.nav.active_data::<Page>(),
                Some(Page::Comics | Page::Volumes | Page::Publishers)
            )
    }

    fn load_active_page(&self) -> Task<Message> {
        match self
            .nav
            .active_data::<Page>()
            .copied()
            .unwrap_or(Page::Comics)
        {
            Page::Comics => self.load_comics(),
            Page::Volumes => self.load_volumes(),
            Page::Publishers => self.load_publishers(),
            Page::ComicVine => Task::none(),
            Page::Downloads => Task::none(),
        }
    }

    fn update_title(&mut self) -> Task<Message> {
        let page = self.nav.text(self.nav.active()).unwrap_or("Biblioteca");
        let title = format!("Babelcomics — {page}");
        self.core
            .main_window_id()
            .map_or_else(Task::none, |id| self.set_window_title(title, id))
    }

    fn fail(&mut self, error: String) {
        self.loading = false;
        self.error = Some(error);
    }

    async fn resolve_issue_covers(requests: Vec<CoverRequest>) -> HashMap<i64, Vec<u8>> {
        let concurrency = Arc::new(tokio::sync::Semaphore::new(8));
        let mut tasks = tokio::task::JoinSet::new();
        for request in requests {
            let concurrency = concurrency.clone();
            tasks.spawn(async move {
                let _permit = concurrency.acquire_owned().await.ok()?;
                let bytes = read_comicbook_info_cover_bytes(
                    request.ruta_local.as_deref(),
                    request.url_original.as_deref(),
                    &request.volume_name,
                    request.volume_id,
                )
                .await?;
                Some((request.id, bytes))
            });
        }
        let mut covers = HashMap::new();
        while let Some(result) = tasks.join_next().await {
            if let Ok(Some((id, bytes))) = result {
                covers.insert(id, bytes);
            }
        }
        covers
    }

    fn load_comics(&self) -> Task<Message> {
        let pool = self.pool.clone();
        let query = self.search.trim().to_owned();
        let catalog_filter = self.catalog_filter;
        Task::perform(
            async move {
                let repo = ComicbookRepository::new(&pool);
                let filter = ComicFilter {
                    query: (!query.is_empty()).then_some(query),
                    clasificado: match catalog_filter {
                        CatalogFilter::All => None,
                        CatalogFilter::Catalogued => Some(true),
                        CatalogFilter::Uncatalogued => Some(false),
                    },
                    ..ComicFilter::default()
                };
                let items = repo
                    .get_page(PAGE_LIMIT, 0, false, Some(&filter))
                    .await
                    .map_err(|e| e.to_string())?;
                let total = repo
                    .count_filtered(&filter)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok((items, total))
            },
            |result| cosmic::Action::App(Message::ComicsLoaded(result)),
        )
    }

    fn save_api_settings(&self) -> Task<Message> {
        let pool = self.pool.clone();
        let api_key = self.api_key_input.trim().to_string();
        let api_url = self.api_url_input.trim().to_string();
        Task::perform(
            async move {
                let repository = SetupRepository::new(&pool);
                let mut setup = repository.get().await.map_err(|error| error.to_string())?;
                setup.api_key_encrypted = (!api_key.is_empty()).then_some(api_key);
                setup.api_url = (!api_url.is_empty()).then_some(api_url);
                repository
                    .save(&setup)
                    .await
                    .map_err(|error| error.to_string())
            },
            |result| cosmic::Action::App(Message::ApiSettingsSaved(result)),
        )
    }

    fn test_comicvine_connection(&self) -> Task<Message> {
        let api_key = self.api_key_input.trim().to_string();
        let api_url = self.api_url_input.trim().to_string();
        Task::perform(
            async move {
                let client =
                    ComicVineClient::new(api_key, (!api_url.is_empty()).then_some(api_url))
                        .map_err(|error| error.to_string())?;
                client.validate().await.map_err(|error| error.to_string())
            },
            |result| cosmic::Action::App(Message::ComicVineConnectionTested(result)),
        )
    }

    fn search_comicvine(&self) -> Task<Message> {
        let pool = self.pool.clone();
        let query = self.comicvine_query.trim().to_string();
        let mode = self.comicvine_search_mode;
        Task::perform(
            async move {
                let setup = SetupRepository::new(&pool)
                    .get()
                    .await
                    .map_err(|error| error.to_string())?;
                let api_key = setup
                    .api_key_encrypted
                    .filter(|key| !key.is_empty())
                    .ok_or_else(|| {
                        "Configura y guarda primero la API key de ComicVine".to_string()
                    })?;
                let client = ComicVineClient::new(api_key, setup.api_url)
                    .map_err(|error| error.to_string())?;
                Ok(match mode {
                    ComicVineSearchMode::Volumes => {
                        EitherComicVineResults::Volumes(client.search_volumes(&query, 60).await)
                    }
                    ComicVineSearchMode::Publishers => EitherComicVineResults::Publishers(
                        client.get_publishers(60, 0, Some(&query)).await,
                    ),
                })
            },
            move |result| {
                cosmic::Action::App(match result {
                    Ok(EitherComicVineResults::Volumes(results)) => {
                        Message::ComicVineSearchFinished(Ok(results))
                    }
                    Ok(EitherComicVineResults::Publishers(results)) => {
                        Message::ComicVinePublisherSearchFinished(Ok(results))
                    }
                    Err(error) => match mode {
                        ComicVineSearchMode::Volumes => {
                            Message::ComicVineSearchFinished(Err(error))
                        }
                        ComicVineSearchMode::Publishers => {
                            Message::ComicVinePublisherSearchFinished(Err(error))
                        }
                    },
                })
            },
        )
    }

    fn import_comicvine_publishers(&self) -> Task<Message> {
        let pool = self.pool.clone();
        let publisher_ids = self
            .comicvine_publishers_selected
            .iter()
            .copied()
            .collect::<Vec<_>>();
        Task::perform(
            async move {
                let setup = SetupRepository::new(&pool)
                    .get()
                    .await
                    .map_err(|error| error.to_string())?;
                let api_key = setup
                    .api_key_encrypted
                    .filter(|key| !key.is_empty())
                    .ok_or_else(|| {
                        "Configura y guarda primero la API key de ComicVine".to_string()
                    })?;
                let client = ComicVineClient::new(api_key, setup.api_url)
                    .map_err(|error| error.to_string())?;
                let mut names = Vec::with_capacity(publisher_ids.len());
                let mut inserted = 0;
                let mut skipped = 0;

                for publisher_id in publisher_ids {
                    let report = publisher_import_service::import_publisher_from_cv(
                        &pool,
                        &client,
                        publisher_id,
                    )
                    .await
                    .map_err(|error| error.to_string())?;
                    names.push(report.publisher_name);
                    inserted += report.volumes_inserted;
                    skipped += report.volumes_skipped;
                }

                Ok((names, inserted, skipped))
            },
            |result| cosmic::Action::App(Message::ComicVinePublishersImported(result)),
        )
    }

    fn load_volumes(&self) -> Task<Message> {
        let pool = self.pool.clone();
        let query = self.search.trim().to_owned();
        let catalog_filter = self.catalog_filter;
        let update_filter = self.volume_update_filter;
        let sort_order = self.volume_sort_order;
        Task::perform(
            async move {
                let sort_sql = sort_order.to_sql();
                let parsed = parse_search_query(&query);
                let mut search_parts = Vec::new();
                let mut search_binds = Vec::new();
                for word in parsed.must_include {
                    search_parts.push("AND (v.nombre LIKE ? OR v.deck LIKE ?)");
                    let pattern = format!("%{word}%");
                    search_binds.extend([pattern.clone(), pattern]);
                }
                for word in parsed.must_exclude {
                    search_parts.push("AND (v.nombre NOT LIKE ? AND v.deck NOT LIKE ?)");
                    let pattern = format!("%{word}%");
                    search_binds.extend([pattern.clone(), pattern]);
                }
                let catalog_sql = match catalog_filter {
                    CatalogFilter::All => "",
                    CatalogFilter::Catalogued => {
                        "AND EXISTS (SELECT 1 FROM comicbooks_info ci JOIN comicbooks cb ON cb.id_comicbook_info = ci.id_comicbook_info WHERE ci.id_volume = v.id_volume)"
                    }
                    CatalogFilter::Uncatalogued => {
                        "AND NOT EXISTS (SELECT 1 FROM comicbooks_info ci JOIN comicbooks cb ON cb.id_comicbook_info = ci.id_comicbook_info WHERE ci.id_volume = v.id_volume)"
                    }
                };
                let update_sql = match update_filter {
                    VolumeUpdateFilter::All => "",
                    VolumeUpdateFilter::Updated => {
                        "AND EXISTS (SELECT 1 FROM comicbooks_info ci WHERE ci.id_volume = v.id_volume AND ci.id_comicvine IS NOT NULL)"
                    }
                    VolumeUpdateFilter::NotUpdated => {
                        "AND NOT EXISTS (SELECT 1 FROM comicbooks_info ci WHERE ci.id_volume = v.id_volume AND ci.id_comicvine IS NOT NULL)"
                    }
                };
                let sql = format!(
                    "SELECT v.id_volume, v.nombre, COALESCE(v.anio_inicio, 0), COALESCE(v.cantidad_numeros, 0), (SELECT COUNT(*) FROM comicbooks_info ci JOIN comicbooks cb ON cb.id_comicbook_info = ci.id_comicbook_info WHERE ci.id_volume = v.id_volume) AS cantidad_poseida, (SELECT cb.id_comicbook FROM comicbooks cb JOIN comicbooks_info ci ON cb.id_comicbook_info = ci.id_comicbook_info WHERE ci.id_volume = v.id_volume ORDER BY CAST(ci.numero AS REAL) ASC LIMIT 1), (SELECT cb.path FROM comicbooks cb JOIN comicbooks_info ci ON cb.id_comicbook_info = ci.id_comicbook_info WHERE ci.id_volume = v.id_volume ORDER BY CAST(ci.numero AS REAL) ASC LIMIT 1), COALESCE(v.image_url, ''), EXISTS (SELECT 1 FROM comicbooks_info ci WHERE ci.id_volume = v.id_volume AND ci.id_comicvine IS NOT NULL) FROM volumens v WHERE 1=1 {} {} {} ORDER BY {} LIMIT ?",
                    search_parts.join(" "),
                    catalog_sql,
                    update_sql,
                    sort_sql,
                );
                let mut statement = sqlx::query(&sql);
                for bind in search_binds {
                    statement = statement.bind(bind);
                }
                let rows = statement
                    .bind(PAGE_LIMIT)
                    .fetch_all(&pool)
                    .await
                    .map_err(|e| e.to_string())?;
                use sqlx::Row;
                Ok(rows
                    .into_iter()
                    .map(|row| VolumeListItem {
                        volume: VolumeView {
                            id_volume: row.get(0),
                            nombre: row.get(1),
                            anio_inicio: row.get(2),
                            cantidad_numeros: row.get(3),
                            cantidad_poseida: row.get(4),
                            id_comicbook_portada: row.get(5),
                            path_portada: row.get(6),
                            image_url: row.get(7),
                        },
                        // No usar `fue_actualizado_api`: ese flag corresponde al
                        // detalle individual del issue, no al listado del volumen.
                        has_api_updates: row.get(8),
                    })
                    .collect())
            },
            |result| cosmic::Action::App(Message::VolumesLoaded(result)),
        )
    }

    fn load_filter_preferences(&self) -> Task<Message> {
        let pool = self.pool.clone();
        Task::perform(
            async move {
                let setup = SetupRepository::new(&pool)
                    .get()
                    .await
                    .map_err(|error| error.to_string())?;
                let catalog = match setup.catalog_filter.as_str() {
                    "catalogued" => CatalogFilter::Catalogued,
                    "uncatalogued" => CatalogFilter::Uncatalogued,
                    _ => CatalogFilter::All,
                };
                let updates = match setup.volume_update_filter.as_str() {
                    "updated" => VolumeUpdateFilter::Updated,
                    "not_updated" => VolumeUpdateFilter::NotUpdated,
                    _ => VolumeUpdateFilter::All,
                };
                Ok((catalog, updates))
            },
            |result| cosmic::Action::App(Message::FilterPreferencesLoaded(result)),
        )
    }

    fn save_filter_preferences(&self) -> Task<Message> {
        let pool = self.pool.clone();
        let catalog = match self.catalog_filter {
            CatalogFilter::All => "all",
            CatalogFilter::Catalogued => "catalogued",
            CatalogFilter::Uncatalogued => "uncatalogued",
        };
        let updates = match self.volume_update_filter {
            VolumeUpdateFilter::All => "all",
            VolumeUpdateFilter::Updated => "updated",
            VolumeUpdateFilter::NotUpdated => "not_updated",
        };
        Task::perform(
            async move {
                SetupRepository::new(&pool)
                    .set_library_filters(catalog, updates)
                    .await
                    .map_err(|error| error.to_string())?;
                Ok(())
            },
            |result| cosmic::Action::App(Message::FilterPreferencesSaved(result)),
        )
    }

    fn load_volume_sort_order(&self) -> Task<Message> {
        let pool = self.pool.clone();
        Task::perform(
            async move {
                let setup = SetupRepository::new(&pool)
                    .get()
                    .await
                    .map_err(|error| error.to_string())?;
                Ok(VolumeSortOrder::from_db(&setup.volume_sort_order))
            },
            |result| cosmic::Action::App(Message::VolumeSortOrderLoaded(result)),
        )
    }

    fn save_volume_sort_order(&self) -> Task<Message> {
        let pool = self.pool.clone();
        let sort_order = self.volume_sort_order.to_db().to_string();
        Task::perform(
            async move {
                SetupRepository::new(&pool)
                    .set_volume_sort_order(&sort_order)
                    .await
                    .map_err(|error| error.to_string())?;
                Ok(())
            },
            |result| cosmic::Action::App(Message::VolumeSortOrderSaved(result)),
        )
    }

    fn load_volume_detail(&self, volume_id: i64) -> Task<Message> {
        let pool = self.pool.clone();
        Task::perform(
            async move {
                let volume = VolumeRepository::new(&pool)
                    .get_by_id(volume_id)
                    .await
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| "No se encontró la serie".to_string())?;
                let publisher = PublisherRepository::new(&pool)
                    .get_by_id(volume.id_publisher)
                    .await
                    .map_err(|error| error.to_string())?;
                let issue_repository = ComicbookInfoRepository::new(&pool);
                let issues = issue_repository
                    .get_view_by_volume(volume_id)
                    .await
                    .map_err(|error| error.to_string())?;
                let mut cover_requests = Vec::new();
                let mut cover_order = Vec::new();
                let covers = issue_repository
                    .get_covers_by_volume(volume_id)
                    .await
                    .map_err(|error| error.to_string())?;
                for cover in covers {
                    cover_order.push((cover.id, cover.id_comicbook_info));
                    cover_requests.push(CoverRequest {
                        id: cover.id,
                        ruta_local: cover.ruta_local,
                        url_original: Some(cover.url_original),
                        volume_name: volume.nombre.clone(),
                        volume_id,
                    });
                }
                let mut raw_covers = Self::resolve_issue_covers(cover_requests).await;
                let missing_ids = issues
                    .iter()
                    .filter(|issue| issue.physical_count == 0)
                    .map(|issue| issue.info.id_comicbook_info)
                    .collect::<BTreeSet<_>>();
                let issue_covers = tokio::task::spawn_blocking(move || {
                    let mut prepared = HashMap::<i64, Vec<ImageHandle>>::new();
                    for (cover_id, issue_id) in cover_order {
                        let Some(bytes) = raw_covers.remove(&cover_id) else {
                            continue;
                        };
                        let handle = if missing_ids.contains(&issue_id) {
                            image::load_from_memory(&bytes)
                                .map(|image| {
                                    let rgba = image.grayscale().thumbnail(400, 480).to_rgba8();
                                    let (width, height) = rgba.dimensions();
                                    ImageHandle::from_rgba(width, height, rgba.into_raw())
                                })
                                .unwrap_or_else(|_| ImageHandle::from_bytes(bytes))
                        } else {
                            ImageHandle::from_bytes(bytes)
                        };
                        prepared.entry(issue_id).or_default().push(handle);
                    }
                    prepared
                })
                .await
                .map_err(|error| format!("No se pudieron preparar las portadas: {error}"))?;
                let (clip_total, clip_local, clip_indexed, _) = issue_repository
                    .count_clip_index_stats(Some(volume_id))
                    .await
                    .map_err(|error| error.to_string())?;
                let clip_pending = issue_repository
                    .get_covers_for_clip(Some(volume_id), true)
                    .await
                    .map_err(|error| error.to_string())?
                    .len() as i64;
                Ok(VolumeDetailData {
                    volume,
                    publisher,
                    issues,
                    clip_total,
                    clip_local,
                    clip_indexed,
                    clip_pending,
                    issue_covers,
                })
            },
            |result| cosmic::Action::App(Message::VolumeDetailLoaded(Box::new(result))),
        )
    }

    fn open_selected_issue_detail(&mut self) -> Task<Message> {
        let Some(volume) = self.volume_detail.as_ref() else {
            return Task::none();
        };
        let issues = volume
            .issues
            .iter()
            .filter(|issue| self.selected_issues.contains(&issue.info.id_comicbook_info))
            .cloned()
            .collect::<Vec<_>>();
        if issues.is_empty() {
            return Task::none();
        }
        let issue_covers = issues
            .iter()
            .filter_map(|issue| {
                volume
                    .issue_covers
                    .get(&issue.info.id_comicbook_info)
                    .cloned()
                    .map(|covers| (issue.info.id_comicbook_info, covers))
            })
            .collect();
        self.issue_detail = None;
        self.issue_detail_loading = true;
        self.issue_detail_error = None;
        self.issue_detail_tab = IssueDetailTab::Information;
        let pool = self.pool.clone();
        Task::perform(
            async move {
                let repository = ComicbookRepository::new(&pool);
                let mut physical_comics = Vec::new();
                for issue in &issues {
                    let comics = repository
                        .get_by_info_id(issue.info.id_comicbook_info)
                        .await
                        .map_err(|error| error.to_string())?;
                    for comic in comics {
                        if let Some(comic) = repository
                            .get_view_by_id(comic.id_comicbook)
                            .await
                            .map_err(|error| error.to_string())?
                        {
                            physical_comics.push((issue.info.id_comicbook_info, comic));
                        }
                    }
                }
                physical_comics.sort_by(|left, right| left.1.path.cmp(&right.1.path));
                Ok(IssueDetailData {
                    issues,
                    physical_comics,
                    issue_covers,
                })
            },
            |result| cosmic::Action::App(Message::IssueDetailLoaded(Box::new(result))),
        )
    }

    fn close_issue_detail(&mut self) {
        self.issue_detail = None;
        self.issue_detail_loading = false;
        self.issue_detail_error = None;
        self.issue_detail_tab = IssueDetailTab::Information;
        self.reader_error = None;
    }

    fn unlink_physical_comic(&self, comic_id: i64) -> Task<Message> {
        let pool = self.pool.clone();
        Task::perform(
            async move {
                ComicbookRepository::new(&pool)
                    .set_info(comic_id, None)
                    .await
                    .map_err(|error| error.to_string())
            },
            move |result| cosmic::Action::App(Message::PhysicalComicUnlinked(comic_id, result)),
        )
    }

    fn import_pending_reading_progress(&self) -> Task<Message> {
        let pool = self.pool.clone();
        Task::perform(
            async move {
                ReadingProgressRepository::new(&pool)
                    .import_pending()
                    .await
                    .map_err(|error| error.to_string())
            },
            |result| cosmic::Action::App(Message::PendingReadingProgressImported(result)),
        )
    }

    fn launch_reader(&self, comic_id: i64, comic_path: String) -> Task<Message> {
        let pool = self.pool.clone();
        Task::perform(
            async move {
                let path = std::path::PathBuf::from(&comic_path);
                if !path.is_file() {
                    return Err(format!("No se encontró el archivo: {}", path.display()));
                }
                let setup = SetupRepository::new(&pool)
                    .get()
                    .await
                    .map_err(|error| format!("No se pudo leer la configuración: {error}"))?;
                let reader_options = [
                    (
                        "--fit",
                        ReaderFit::from_db(&setup.reader_fit).as_str().to_string(),
                    ),
                    (
                        "--direction",
                        ReaderDirection::from_db(&setup.reader_direction)
                            .as_str()
                            .to_string(),
                    ),
                    (
                        "--guided",
                        ReaderGuided::from_db(&setup.reader_guided)
                            .as_str()
                            .to_string(),
                    ),
                    ("--filter", setup.reader_filter.to_string()),
                ];
                let configured_reader = setup.reader_path.filter(|path| !path.trim().is_empty());

                let repository = ReadingProgressRepository::new(&pool);
                let progress_path = repository
                    .prepare_spool(comic_id, &path)
                    .await
                    .map_err(|error| format!("No se pudo preparar el progreso: {error}"))?;

                let command_path = path.clone();
                let command_progress_path = progress_path.clone();
                let process_result = tokio::task::spawn_blocking(move || {
                    let mut executables = Vec::<std::path::PathBuf>::new();
                    // La ruta elegida en Preferencias tiene prioridad sobre la búsqueda automática.
                    if let Some(configured) = configured_reader {
                        executables.push(configured.into());
                    }
                    if let Some(configured) = std::env::var_os("BABELCOMICS_READER") {
                        executables.push(configured.into());
                    }
                    if let Ok(current_exe) = std::env::current_exe()
                        && let Some(directory) = current_exe.parent()
                    {
                        executables.push(directory.join("cosmic-reader"));
                    }
                    // Both crates share the workspace target directory, which may
                    // be redirected with CARGO_TARGET_DIR at build time.
                    let development_root = option_env!("CARGO_TARGET_DIR")
                        .map(std::path::PathBuf::from)
                        .unwrap_or_else(|| {
                            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../target")
                        });
                    executables.push(development_root.join("release/cosmic-reader"));
                    executables.push(development_root.join("debug/cosmic-reader"));
                    executables.push(std::path::PathBuf::from("cosmic-reader"));

                    let mut failures = Vec::new();
                    for executable in executables {
                        let mut command = std::process::Command::new(&executable);
                        for (name, value) in &reader_options {
                            command.arg(name).arg(value);
                        }
                        match command
                            .arg("--progress-file")
                            .arg(&command_progress_path)
                            .arg(&command_path)
                            .spawn()
                        {
                            Ok(mut child) => return child.wait().map_err(|error| error.to_string()),
                            Err(error) => failures.push(format!(
                                "{}: {error}",
                                executable.to_string_lossy()
                            )),
                        }
                    }
                    Err(format!(
                        "No se pudo iniciar cosmic-reader. Instálalo junto a Babelcomics, agrégalo al PATH o define BABELCOMICS_READER. {}",
                        failures.join(" · ")
                    ))
                })
                .await
                .map_err(|error| format!("Falló el proceso del lector: {error}"))?;

                let status = match process_result {
                    Ok(status) => status,
                    Err(error) => {
                        let _ = tokio::fs::remove_file(&progress_path).await;
                        return Err(error);
                    }
                };
                let progress = repository
                    .import_spool(&progress_path)
                    .await
                    .map_err(|error| format!("No se pudo importar el progreso: {error}"))?;
                if !status.success() {
                    return Err(format!(
                        "El lector terminó con {status}; el progreso alcanzado fue guardado."
                    ));
                }
                Ok(progress)
            },
            move |result| cosmic::Action::App(Message::ReaderFinished(comic_id, result)),
        )
    }

    fn load_catalog_candidates(&self, comic_id: i64) -> Task<Message> {
        let pool = self.pool.clone();
        let job_id = background_jobs::start_job(
            "Catalogación inteligente",
            "Preparando la portada del cómic…",
            JobKind::Clip,
        );
        Task::perform(
            async move {
                let result = async {
                    let repository = ComicbookRepository::new(&pool);
                    if repository
                        .get_clip_embedding(comic_id)
                        .await
                        .map_err(|error| error.to_string())?
                        .is_none()
                    {
                        background_jobs::update_job(
                            &job_id,
                            "Calculando el embedding de la portada…",
                            0.25,
                        );
                        let comic = repository
                            .get_by_id(comic_id)
                            .await
                            .map_err(|error| error.to_string())?
                            .ok_or_else(|| "No se encontró el cómic".to_string())?;
                        let mut cover = None;
                        for size in [CardSize::Large, CardSize::Medium, CardSize::Small] {
                            if let Ok(bytes) =
                                tokio::fs::read(comic_thumbnail_path(comic_id, size)).await
                            {
                                cover = Some(bytes);
                                break;
                            }
                        }
                        let cover = match cover {
                            Some(bytes) => bytes,
                            None => {
                                let path = comic.path;
                                tokio::task::spawn_blocking(move || {
                                    babelcomics_core::helpers::extractor::extract_cover(&path)
                                        .map_err(|error| error.to_string())
                                })
                                .await
                                .map_err(|error| error.to_string())??
                            }
                        };
                        let embedding = tokio::task::spawn_blocking(move || {
                            babelcomics_core::helpers::clip_embedder::embed_image(&cover)
                                .map_err(|error| error.to_string())
                        })
                        .await
                        .map_err(|error| error.to_string())??;
                        repository
                            .set_clip_embedding(
                                comic_id,
                                &babelcomics_core::helpers::clip_embedder::to_bytes(&embedding),
                            )
                            .await
                            .map_err(|error| error.to_string())?;
                    }

                    background_jobs::update_job(
                        &job_id,
                        "Comparando con el índice de portadas…",
                        0.7,
                    );
                    let candidates = suggest_best_matches(&pool, comic_id, 10)
                        .await
                        .map_err(|error| error.to_string())?;
                    let requests = candidates
                        .iter()
                        .map(|candidate| CoverRequest {
                            id: candidate.id_comicbook_info,
                            ruta_local: candidate.ruta_cover.clone(),
                            url_original: candidate.url_original.clone(),
                            volume_name: candidate.nombre_volume.clone().unwrap_or_default(),
                            volume_id: candidate.id_volume.unwrap_or_default(),
                        })
                        .collect();
                    let raw_covers = Self::resolve_issue_covers(requests).await;
                    let covers = raw_covers
                        .into_iter()
                        .map(|(id, bytes)| (id, ImageHandle::from_bytes(bytes)))
                        .collect();
                    Ok((candidates, covers))
                }
                .await;

                match &result {
                    Ok((candidates, _)) => background_jobs::finish_job(
                        &job_id,
                        format!("{} candidatos encontrados", candidates.len()),
                        false,
                    ),
                    Err(error) => background_jobs::finish_job(&job_id, error, true),
                }
                result
            },
            |result| cosmic::Action::App(Message::CatalogLoaded(result)),
        )
    }

    fn apply_catalog_candidate(
        &self,
        comic_id: i64,
        candidate: UnifiedSuggestion,
        selected_index: usize,
    ) -> Task<Message> {
        let pool = self.pool.clone();
        let best_similarity = self
            .catalog_candidates
            .first()
            .map(|candidate| candidate.similarity)
            .unwrap_or(candidate.similarity);
        Task::perform(
            async move {
                let method = match candidate.method {
                    SuggestionMethod::Clip => "clip",
                    SuggestionMethod::Hash => "hash",
                };
                ComicbookRepository::new(&pool)
                    .set_info_with_match_metrics(
                        comic_id,
                        candidate.id_comicbook_info,
                        candidate.similarity as f64,
                        best_similarity as f64,
                        selected_index as i64 + 1,
                        method,
                    )
                    .await
                    .map_err(|error| error.to_string())
            },
            |result| cosmic::Action::App(Message::CatalogApplied(result)),
        )
    }

    fn load_manual_catalog_volumes(&self) -> Task<Message> {
        let pool = self.pool.clone();
        let query = self.manual_catalog_query.trim().to_string();
        Task::perform(
            async move {
                VolumeRepository::new(&pool)
                    .get_page(
                        80,
                        0,
                        (!query.is_empty()).then_some(query.as_str()),
                        VolumeSortOrder::NombreAsc,
                        &[],
                    )
                    .await
                    .map_err(|error| error.to_string())
            },
            |result| cosmic::Action::App(Message::ManualCatalogVolumesLoaded(result)),
        )
    }

    fn load_manual_catalog_issues(&self, volume_id: i64) -> Task<Message> {
        let pool = self.pool.clone();
        let volume_name = self
            .manual_catalog_volume
            .as_ref()
            .map(|volume| volume.nombre.clone())
            .unwrap_or_default();
        Task::perform(
            async move {
                let issues = ComicbookInfoRepository::new(&pool)
                    .get_view_by_volume(volume_id)
                    .await
                    .map_err(|error| error.to_string())?;
                let requests = issues
                    .iter()
                    .map(|issue| CoverRequest {
                        id: issue.info.id_comicbook_info,
                        ruta_local: issue.ruta_cover.clone(),
                        url_original: issue.url_original.clone(),
                        volume_name: volume_name.clone(),
                        volume_id,
                    })
                    .collect();
                let candidates = issues
                    .into_iter()
                    .map(|issue| UnifiedSuggestion {
                        id_comicbook_info: issue.info.id_comicbook_info,
                        titulo: issue.info.titulo,
                        numero: issue.info.numero,
                        id_volume: issue.info.id_volume,
                        nombre_volume: Some(volume_name.clone()),
                        similarity: 0.0,
                        // El método no se muestra en selección manual, pero el tipo
                        // unificado requiere un valor para reutilizar la misma vista.
                        method: SuggestionMethod::Hash,
                        ruta_cover: issue.ruta_cover,
                        url_original: issue.url_original,
                        id_comicvine: issue.info.id_comicvine,
                    })
                    .collect();
                let covers = Self::resolve_issue_covers(requests)
                    .await
                    .into_iter()
                    .map(|(id, bytes)| (id, ImageHandle::from_bytes(bytes)))
                    .collect();
                Ok((candidates, covers))
            },
            |result| cosmic::Action::App(Message::ManualCatalogIssuesLoaded(result)),
        )
    }

    fn apply_manual_catalog(&self, comic_id: i64, issue_id: i64) -> Task<Message> {
        let pool = self.pool.clone();
        Task::perform(
            async move {
                ComicbookRepository::new(&pool)
                    .set_info(comic_id, Some(issue_id))
                    .await
                    .map_err(|error| error.to_string())
            },
            |result| cosmic::Action::App(Message::CatalogApplied(result)),
        )
    }

    fn reset_manual_catalog(&mut self) {
        self.catalog_manual = false;
        self.manual_catalog_query.clear();
        self.manual_catalog_volumes.clear();
        self.manual_catalog_volume = None;
        self.manual_catalog_loading = false;
        self.manual_catalog_error = None;
    }

    fn start_catalog_queue(&mut self, comics: Vec<ComicbookView>) -> Task<Message> {
        self.drawer = None;
        self.core.window.show_context = false;
        self.selected_comics.clear();
        self.selection_anchor = None;
        self.catalog_candidates.clear();
        self.catalog_candidate_covers.clear();
        self.catalog_selected = 0;
        self.catalog_queue = comics;
        self.catalog_queue_index = 0;
        self.catalog_completed = 0;
        self.catalog_skipped = 0;
        self.catalog_loading = true;
        self.catalog_applying = false;
        self.catalog_error = None;
        self.reset_manual_catalog();
        self.search_active = false;
        self.search.clear();
        let comic = self.catalog_queue[0].clone();
        let comic_id = comic.id_comicbook;
        self.catalog_comic = Some(comic);
        self.load_catalog_candidates(comic_id)
    }

    fn advance_catalog_queue(&mut self) -> Option<Task<Message>> {
        let next_index = self.catalog_queue_index + 1;
        if next_index >= self.catalog_queue.len() {
            return None;
        }
        self.catalog_queue_index = next_index;
        let comic = self.catalog_queue[next_index].clone();
        let comic_id = comic.id_comicbook;
        self.catalog_comic = Some(comic);
        self.catalog_candidates.clear();
        self.catalog_candidate_covers.clear();
        self.catalog_selected = 0;
        self.catalog_loading = true;
        self.catalog_applying = false;
        self.catalog_error = None;
        self.reset_manual_catalog();
        Some(self.load_catalog_candidates(comic_id))
    }

    fn clear_catalog(&mut self) {
        self.catalog_comic = None;
        self.catalog_queue.clear();
        self.catalog_queue_index = 0;
        self.catalog_completed = 0;
        self.catalog_skipped = 0;
        self.catalog_candidates.clear();
        self.catalog_candidate_covers.clear();
        self.catalog_selected = 0;
        self.catalog_loading = false;
        self.catalog_applying = false;
        self.catalog_error = None;
        self.reset_manual_catalog();
    }

    fn load_publishers(&self) -> Task<Message> {
        let pool = self.pool.clone();
        let query = self.search.trim().to_owned();
        Task::perform(
            async move {
                let repo = PublisherRepository::new(&pool);
                if query.is_empty() {
                    repo.get_all().await
                } else {
                    repo.search(&query).await
                }
                .map_err(|e| e.to_string())
            },
            |result| cosmic::Action::App(Message::PublishersLoaded(result)),
        )
    }

    fn load_stats(&self) -> Task<Message> {
        let pool = self.pool.clone();
        Task::perform(
            async move {
                let comics = ComicbookRepository::new(&pool);
                let setup_repository = SetupRepository::new(&pool);
                let issue_repository = ComicbookInfoRepository::new(&pool);
                let setup = setup_repository.get().await.map_err(|e| e.to_string())?;
                let scan_extensions = setup_repository
                    .get_scan_extensions()
                    .await
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .collect();
                let (clip_total, clip_local, clip_indexed, clip_pending_local) = issue_repository
                    .count_clip_index_stats(None)
                    .await
                    .map_err(|e| e.to_string())?;
                let clip_pending = issue_repository
                    .get_covers_for_clip(None, true)
                    .await
                    .map_err(|e| e.to_string())?
                    .len() as i64;
                let stats = LibraryStats {
                    comics: comics.count().await.map_err(|e| e.to_string())?,
                    uncatalogued: comics
                        .count_uncatalogued()
                        .await
                        .map_err(|e| e.to_string())?,
                    errors: comics
                        .count_with_errors()
                        .await
                        .map_err(|e| e.to_string())?,
                    volumes: VolumeRepository::new(&pool)
                        .count()
                        .await
                        .map_err(|e| e.to_string())?,
                    publishers: PublisherRepository::new(&pool)
                        .count()
                        .await
                        .map_err(|e| e.to_string())?,
                    directories: setup_repository
                        .get_directorios()
                        .await
                        .map_err(|e| e.to_string())?,
                    thumbnail_size: setup.thumbnail_size,
                    thumbnails_directory: setup.carpeta_thumbnails,
                    normalized_catalog_directory: setup.carpeta_catalogo_normalizado,
                    items_per_page: setup.items_por_pagina,
                    workers: setup.num_workers,
                    reader_filter: setup.reader_filter,
                    reader_path: setup.reader_path,
                    clip_on_startup: setup.clip_al_arranque,
                    clip_on_download: setup.clip_al_descargar,
                    reader_fit: ReaderFit::from_db(&setup.reader_fit),
                    reader_direction: ReaderDirection::from_db(&setup.reader_direction),
                    reader_guided: ReaderGuided::from_db(&setup.reader_guided),
                    scan_extensions,
                    api_key: setup.api_key_encrypted,
                    api_url: setup
                        .api_url
                        .unwrap_or_else(|| "https://comicvine.gamespot.com/api/".to_string()),
                    clip_total,
                    clip_local,
                    clip_indexed,
                    clip_pending,
                    clip_pending_local,
                };
                Ok(stats)
            },
            |result| cosmic::Action::App(Message::StatsLoaded(Box::new(result))),
        )
    }

    fn save_normalized_catalog_directory(&self, raw_path: String) -> Task<Message> {
        let pool = self.pool.clone();
        Task::perform(
            async move {
                let path = std::path::PathBuf::from(raw_path.trim());
                if !path.is_dir() {
                    return Err(format!("La ruta no es una carpeta: {}", path.display()));
                }
                let canonical = path
                    .canonicalize()
                    .map_err(|error| format!("No se pudo resolver {}: {error}", path.display()))?;
                let canonical = canonical.to_string_lossy().into_owned();
                SetupRepository::new(&pool)
                    .set_carpeta_catalogo_normalizado(Some(&canonical))
                    .await
                    .map_err(|error| error.to_string())?;
                Ok(format!(
                    "Carpeta del catálogo normalizado guardada: {canonical}"
                ))
            },
            |result| cosmic::Action::App(Message::PreferenceDirectorySaved(result)),
        )
    }

    fn save_thumbnails_directory(&self, raw_path: String) -> Task<Message> {
        let pool = self.pool.clone();
        Task::perform(
            async move {
                let path = std::path::PathBuf::from(raw_path.trim());
                if !path.is_dir() {
                    return Err(format!("La ruta no es una carpeta: {}", path.display()));
                }
                let new_path = path
                    .canonicalize()
                    .map_err(|error| format!("No se pudo resolver {}: {error}", path.display()))?;
                let old_path = paths::thumbnails_dir();
                let old_path = old_path.canonicalize().unwrap_or(old_path);
                if new_path != old_path {
                    if new_path.starts_with(&old_path) || old_path.starts_with(&new_path) {
                        return Err(
                            "La carpeta nueva y la actual no pueden estar contenidas una dentro de la otra."
                                .to_string(),
                        );
                    }
                    let destination_has_files = std::fs::read_dir(&new_path)
                        .map_err(|error| {
                            format!("No se pudo leer {}: {error}", new_path.display())
                        })?
                        .next()
                        .is_some();
                    if destination_has_files {
                        return Err(
                            "La nueva carpeta de miniaturas debe estar vacía para migrar los archivos actuales."
                                .to_string(),
                        );
                    }
                    paths::migrate_thumbnails(old_path.clone(), new_path.clone()).map_err(
                        |error| format!("No se pudieron migrar las miniaturas: {error}"),
                    )?;
                }

                let old_path_text = old_path.to_string_lossy().into_owned();
                let new_path_text = new_path.to_string_lossy().into_owned();
                let mut tx = pool.begin().await.map_err(|error| error.to_string())?;
                sqlx::query(
                    r#"UPDATE comicbooks_info_covers
                       SET ruta_local = ? || substr(ruta_local, length(?) + 1)
                       WHERE ruta_local IS NOT NULL
                         AND substr(ruta_local, 1, length(?)) = ?"#,
                )
                .bind(&new_path_text)
                .bind(&old_path_text)
                .bind(&old_path_text)
                .bind(&old_path_text)
                .execute(&mut *tx)
                .await
                .map_err(|error| error.to_string())?;
                sqlx::query("UPDATE setups SET carpeta_thumbnails = ? WHERE setupkey = 'default'")
                    .bind(&new_path_text)
                    .execute(&mut *tx)
                    .await
                    .map_err(|error| error.to_string())?;
                tx.commit().await.map_err(|error| error.to_string())?;

                paths::set_thumbnails_base(new_path);
                paths::ensure_thumbnail_dirs()
                    .map_err(|error| format!("No se pudieron crear las carpetas: {error}"))?;
                Ok(format!(
                    "Miniaturas migradas a la nueva carpeta: {new_path_text}"
                ))
            },
            |result| cosmic::Action::App(Message::PreferenceDirectorySaved(result)),
        )
    }

    fn prepare_organization(&self) -> Task<Message> {
        let pool = self.pool.clone();
        let destination = self
            .stats
            .normalized_catalog_directory
            .clone()
            .unwrap_or_default();
        Task::perform(
            async move {
                if destination.is_empty() {
                    return Err("No hay una carpeta de catálogo configurada.".to_string());
                }
                let organizer = ComicOrganizer::new(destination, pool);
                organizer
                    .create_organization_plan(None, None)
                    .await
                    .map_err(|error| error.to_string())
            },
            |result| cosmic::Action::App(Message::OrganizationPlanReady(result)),
        )
    }

    fn execute_organization(&self) -> Task<Message> {
        let pool = self.pool.clone();
        let destination = self
            .stats
            .normalized_catalog_directory
            .clone()
            .unwrap_or_default();
        let plans = self.organization_plan.clone();
        let job_id = background_jobs::start_job(
            "Organizar catálogo",
            "Preparando movimiento de archivos…",
            JobKind::Import,
        );
        Task::perform(
            async move {
                if destination.is_empty() {
                    background_jobs::finish_job(
                        &job_id,
                        "No hay una carpeta de catálogo configurada",
                        true,
                    );
                    return Err("No hay una carpeta de catálogo configurada.".to_string());
                }
                let organizer = ComicOrganizer::new(destination, pool);
                let progress_job_id = job_id.clone();
                let stats = organizer
                    .execute_plans(
                        &plans,
                        false,
                        move |index, total, plan, success, message| {
                            let progress = (index + 1) as f64 / total.max(1) as f64;
                            let state = if success { "Movido" } else { "Omitido" };
                            background_jobs::update_job(
                                &progress_job_id,
                                format!(
                                    "{state} {}/{}: {} — {message}",
                                    index + 1,
                                    total,
                                    plan.filename_normalized
                                ),
                                progress,
                            );
                        },
                    )
                    .await;
                let summary = format!(
                    "{} movidos · {} omitidos · {} errores",
                    stats.success, stats.skipped, stats.failed
                );
                background_jobs::finish_job(&job_id, &summary, stats.failed > 0);
                Ok((stats.success, stats.failed, stats.skipped))
            },
            |result| cosmic::Action::App(Message::OrganizationFinished(result)),
        )
    }

    fn save_directories(&mut self, paths: Vec<String>) -> Task<Message> {
        self.directory_saving = true;
        self.directory_error = None;
        let pool = self.pool.clone();
        Task::perform(
            async move {
                let mut normalized = Vec::with_capacity(paths.len());
                for raw_path in paths {
                    let path = std::path::PathBuf::from(raw_path.trim());
                    if !path.exists() {
                        return Err(format!("La ruta no existe: {}", path.display()));
                    }
                    if !path.is_dir() {
                        return Err(format!("La ruta no es una carpeta: {}", path.display()));
                    }
                    let canonical = path.canonicalize().map_err(|error| {
                        format!("No se pudo resolver {}: {error}", path.display())
                    })?;
                    normalized.push(canonical.to_string_lossy().into_owned());
                }

                let repository = SetupRepository::new(&pool);
                for path in normalized {
                    repository
                        .add_directorio(&path)
                        .await
                        .map_err(|error| error.to_string())?;
                }
                Ok(())
            },
            |result| cosmic::Action::App(Message::DirectoriesSaved(result)),
        )
    }

    fn run_scan(&self) -> Task<Message> {
        let pool = self.pool.clone();
        let job_id = background_jobs::start_job(
            "Escaneo de biblioteca",
            "Leyendo las carpetas configuradas…",
            JobKind::Scan,
        );

        Task::perform(
            async move {
                let result = async {
                    let setup_repository = SetupRepository::new(&pool);
                    let directories = setup_repository
                        .get_directorios()
                        .await
                        .map_err(|error| error.to_string())?;
                    let paths: Vec<String> = directories
                        .into_iter()
                        .map(|directory| directory.path)
                        .collect();
                    if paths.is_empty() {
                        return Err("No hay carpetas configuradas para escanear".to_string());
                    }

                    background_jobs::update_job(
                        &job_id,
                        format!("Escaneando {} carpeta(s)…", paths.len()),
                        0.1,
                    );
                    let setup = setup_repository
                        .get()
                        .await
                        .map_err(|error| error.to_string())?;
                    let card_size = CardSize::from_db(setup.thumbnail_size);
                    let extensions = setup_repository
                        .get_scan_extensions()
                        .await
                        .map_err(|error| error.to_string())?;
                    if extensions.is_empty() {
                        return Err(
                            "Selecciona al menos un tipo de archivo para escanear".to_string()
                        );
                    }
                    let scan = scan_service::run_scan_with_extensions(
                        &pool,
                        &paths,
                        card_size,
                        &extensions,
                    )
                    .await
                    .map_err(|error| error.to_string())?;

                    let mut parts = vec![
                        format!("{} cómics encontrados", scan.total_found),
                        format!("{} nuevos", scan.new_inserted),
                        format!("{} miniaturas generadas", scan.covers_generated),
                    ];
                    if !scan.errors.is_empty() {
                        parts.push(format!("{} errores", scan.errors.len()));
                    }
                    Ok((parts.join(" · "), !scan.errors.is_empty()))
                }
                .await;

                match &result {
                    Ok((summary, has_errors)) => {
                        background_jobs::finish_job(&job_id, summary, *has_errors);
                    }
                    Err(error) => background_jobs::finish_job(&job_id, error, true),
                }
                result
            },
            |result| cosmic::Action::App(Message::ScanFinished(result)),
        )
    }

    fn generate_clip_embeddings(&self, volume_id: Option<i64>) -> Task<Message> {
        let pool = self.pool.clone();
        let scope = volume_id
            .map(|_| "del volumen")
            .unwrap_or("de toda la biblioteca");
        let job_id = background_jobs::start_job(
            "Índice visual CLIP",
            format!("Preparando portadas {scope}…"),
            JobKind::Clip,
        );

        Task::perform(
            async move {
                let (progress_tx, progress_rx) = std::sync::mpsc::channel();
                let pool_for_generation = pool.clone();
                let generation = tokio::spawn(async move {
                    scan_service::generate_clip_embeddings(
                        &pool_for_generation,
                        volume_id,
                        true,
                        Some(progress_tx),
                    )
                    .await
                    .map_err(|error| error.to_string())
                });

                while !generation.is_finished() {
                    while let Ok(progress) = progress_rx.try_recv() {
                        let remaining = progress.total.saturating_sub(progress.processed);
                        let fraction = if progress.total > 0 {
                            progress.processed as f64 / progress.total as f64
                        } else {
                            0.0
                        };
                        background_jobs::update_job(
                            &job_id,
                            format!(
                                "{}/{} procesadas · {} restantes · {} generadas · {} errores",
                                progress.processed,
                                progress.total,
                                remaining,
                                progress.generated,
                                progress.errors
                            ),
                            fraction,
                        );
                    }
                    tokio::time::sleep(Duration::from_millis(75)).await;
                }

                while let Ok(progress) = progress_rx.try_recv() {
                    let fraction = if progress.total > 0 {
                        progress.processed as f64 / progress.total as f64
                    } else {
                        1.0
                    };
                    background_jobs::update_job(
                        &job_id,
                        format!(
                            "{}/{} procesadas · {} generadas · {} errores",
                            progress.processed, progress.total, progress.generated, progress.errors
                        ),
                        fraction,
                    );
                }

                let result = generation
                    .await
                    .map_err(|error| format!("Falló la tarea CLIP: {error}"))?;
                match &result {
                    Ok((generated, errors)) => background_jobs::finish_job(
                        &job_id,
                        format!(
                            "{generated} embeddings generados · {} errores",
                            errors.len()
                        ),
                        !errors.is_empty(),
                    ),
                    Err(error) => background_jobs::finish_job(&job_id, error, true),
                }
                result.map(|(generated, errors)| (generated, errors.len()))
            },
            move |result| cosmic::Action::App(Message::ClipGenerationFinished(volume_id, result)),
        )
    }

    fn delete_item(&self, target: DeleteTarget, delete_physical: bool) -> Task<Message> {
        let pool = self.pool.clone();
        Task::perform(
            async move {
                let physical_paths = match target {
                    DeleteTarget::Comic { id, path, .. } => {
                        ComicbookRepository::new(&pool)
                            .delete(id)
                            .await
                            .map_err(|error| error.to_string())?;
                        vec![path]
                    }
                    DeleteTarget::Comics { items, .. } => {
                        let repository = ComicbookRepository::new(&pool);
                        let mut paths = Vec::with_capacity(items.len());
                        for (id, path) in items {
                            repository
                                .delete(id)
                                .await
                                .map_err(|error| error.to_string())?;
                            paths.push(path);
                        }
                        paths
                    }
                    DeleteTarget::Volume { id, .. } => {
                        VolumeRepository::new(&pool)
                            .delete(id)
                            .await
                            .map_err(|error| error.to_string())?;
                        Vec::new()
                    }
                    DeleteTarget::Publisher { id, .. } => {
                        PublisherRepository::new(&pool)
                            .delete(id)
                            .await
                            .map_err(|error| error.to_string())?;
                        Vec::new()
                    }
                };

                if delete_physical && !physical_paths.is_empty() {
                    Self::remove_physical_files(physical_paths).await
                } else {
                    Ok(None)
                }
            },
            |result| cosmic::Action::App(Message::DeleteFinished(result)),
        )
    }

    async fn remove_physical_files(paths: Vec<String>) -> Result<Option<String>, String> {
        let errors = tokio::task::spawn_blocking(move || {
            let mut errors = Vec::new();
            for path in paths {
                match std::fs::remove_file(&path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => errors.push(format!("{}: {error}", path)),
                }
            }
            errors
        })
        .await
        .map_err(|error| format!("Falló la tarea de borrado físico: {error}"))?;

        if errors.is_empty() {
            Ok(None)
        } else {
            let count = errors.len();
            let examples = errors.into_iter().take(3).collect::<Vec<_>>().join(" · ");
            Ok(Some(format!(
                "Se quitaron los registros, pero no se pudieron borrar {count} archivo(s): {examples}"
            )))
        }
    }

    fn select_comic(&mut self, id: i64) {
        if self.shift_pressed {
            let anchor = self
                .selection_anchor
                .filter(|anchor| {
                    self.comics
                        .iter()
                        .any(|comic| comic.id_comicbook == *anchor)
                })
                .unwrap_or(id);
            let anchor_index = self
                .comics
                .iter()
                .position(|comic| comic.id_comicbook == anchor);
            let clicked_index = self
                .comics
                .iter()
                .position(|comic| comic.id_comicbook == id);

            if !self.control_pressed {
                self.selected_comics.clear();
            }
            if let (Some(anchor_index), Some(clicked_index)) = (anchor_index, clicked_index) {
                let start = anchor_index.min(clicked_index);
                let end = anchor_index.max(clicked_index);
                self.selected_comics.extend(
                    self.comics[start..=end]
                        .iter()
                        .map(|comic| comic.id_comicbook),
                );
            } else {
                self.selected_comics.insert(id);
            }
            self.selection_anchor = Some(anchor);
            self.drawer = None;
            self.core.window.show_context = false;
            return;
        }

        if self.control_pressed {
            if !self.selected_comics.insert(id) {
                self.selected_comics.remove(&id);
            }
            self.selection_anchor = Some(id);
            self.drawer = None;
            self.core.window.show_context = false;
            return;
        } else {
            self.selected_comics.clear();
            self.selected_comics.insert(id);
            self.selection_anchor = Some(id);
        }

        // Un clic simple no abre el detalle; si ya estaba abierto, lo actualiza
        // para que muestre el cómic recién seleccionado.
        if matches!(self.drawer, Some(Drawer::Comic(_) | Drawer::Comics(_))) {
            self.show_selected_comics();
        }
    }

    fn show_selected_comics(&mut self) {
        let selected: Vec<_> = self
            .comics
            .iter()
            .filter(|comic| self.selected_comics.contains(&comic.id_comicbook))
            .cloned()
            .collect();
        self.drawer = match selected.as_slice() {
            [] => None,
            [comic] => Some(Drawer::Comic(Box::new(comic.clone()))),
            _ => Some(Drawer::Comics(selected)),
        };
        self.core.window.show_context = self.drawer.is_some();
    }

    fn select_issue(&mut self, id: i64) {
        let Some(detail) = self.volume_detail.as_ref() else {
            return;
        };
        if self.shift_pressed {
            let anchor = self
                .issue_selection_anchor
                .filter(|anchor| {
                    detail
                        .issues
                        .iter()
                        .any(|issue| issue.info.id_comicbook_info == *anchor)
                })
                .unwrap_or(id);
            let anchor_index = detail
                .issues
                .iter()
                .position(|issue| issue.info.id_comicbook_info == anchor);
            let clicked_index = detail
                .issues
                .iter()
                .position(|issue| issue.info.id_comicbook_info == id);
            if !self.control_pressed {
                self.selected_issues.clear();
            }
            if let (Some(anchor_index), Some(clicked_index)) = (anchor_index, clicked_index) {
                let start = anchor_index.min(clicked_index);
                let end = anchor_index.max(clicked_index);
                self.selected_issues.extend(
                    detail.issues[start..=end]
                        .iter()
                        .map(|issue| issue.info.id_comicbook_info),
                );
            } else {
                self.selected_issues.insert(id);
            }
            self.issue_selection_anchor = Some(anchor);
            return;
        }
        if self.control_pressed {
            if !self.selected_issues.insert(id) {
                self.selected_issues.remove(&id);
            }
        } else {
            self.selected_issues.clear();
            self.selected_issues.insert(id);
        }
        self.issue_selection_anchor = Some(id);
    }

    fn select_volume(&mut self, id: i64) -> Task<Message> {
        if self.shift_pressed {
            let anchor = self
                .volume_selection_anchor
                .filter(|anchor| {
                    self.volumes
                        .iter()
                        .any(|item| item.volume.id_volume == *anchor)
                })
                .unwrap_or(id);
            let anchor_index = self
                .volumes
                .iter()
                .position(|item| item.volume.id_volume == anchor);
            let clicked_index = self
                .volumes
                .iter()
                .position(|item| item.volume.id_volume == id);
            if !self.control_pressed {
                self.selected_volumes.clear();
            }
            if let (Some(anchor_index), Some(clicked_index)) = (anchor_index, clicked_index) {
                let start = anchor_index.min(clicked_index);
                let end = anchor_index.max(clicked_index);
                self.selected_volumes.extend(
                    self.volumes[start..=end]
                        .iter()
                        .map(|item| item.volume.id_volume),
                );
            } else {
                self.selected_volumes.insert(id);
            }
            self.volume_selection_anchor = Some(anchor);
            self.drawer = None;
            self.core.window.show_context = false;
            return Task::none();
        }

        if self.control_pressed {
            if !self.selected_volumes.insert(id) {
                self.selected_volumes.remove(&id);
            }
            self.volume_selection_anchor = Some(id);
            self.drawer = None;
            self.core.window.show_context = false;
            return Task::none();
        }

        self.open_volume_detail(id)
    }

    fn open_volume_detail(&mut self, id: i64) -> Task<Message> {
        self.selected_volumes.clear();
        self.volume_selection_anchor = None;
        self.drawer = None;
        self.core.window.show_context = false;
        self.volume_detail = None;
        self.volume_detail_loading = true;
        self.volume_detail_error = None;
        self.volume_detail_tab = VolumeDetailTab::Information;
        self.volume_issue_search.clear();
        self.volume_issue_filter = IssueOwnershipFilter::All;
        self.selected_issues.clear();
        self.issue_selection_anchor = None;
        self.close_issue_detail();
        self.search_active = false;
        self.search.clear();
        self.load_volume_detail(id)
    }

    fn show_selected_volumes(&mut self) {
        let selected = self
            .volumes
            .iter()
            .filter(|item| self.selected_volumes.contains(&item.volume.id_volume))
            .map(|item| item.volume.clone())
            .collect::<Vec<_>>();
        self.drawer = (!selected.is_empty()).then_some(Drawer::Volumes(selected));
        self.core.window.show_context = self.drawer.is_some();
    }

    fn queue_selected_volume_updates(&self) -> Task<Message> {
        let pool = self.pool.clone();
        let volume_ids = self.selected_volumes.iter().copied().collect::<Vec<_>>();
        Task::perform(
            async move {
                let repository = VolumeRepository::new(&pool);
                let manager = DownloadManager::get_instance(pool.clone());
                let mut tracked = Vec::with_capacity(volume_ids.len());
                let mut added = 0usize;
                let mut without_comicvine_id = 0usize;
                for volume_id in volume_ids {
                    let volume = repository
                        .get_by_id(volume_id)
                        .await
                        .map_err(|error| error.to_string())?
                        .ok_or_else(|| format!("No se encontró el volumen {volume_id}"))?;
                    let Some(cv_id) = volume.id_comicvine else {
                        without_comicvine_id += 1;
                        continue;
                    };
                    if manager.add_download(cv_id, &volume.nombre, volume.cantidad_numeros, true) {
                        added += 1;
                    }
                    tracked.push((cv_id, volume_id));
                }
                Ok((tracked, added, without_comicvine_id))
            },
            |result| cosmic::Action::App(Message::SelectedVolumesQueued(result)),
        )
    }

    fn page_shell<'a>(
        &'a self,
        title: &'static str,
        subtitle: String,
        content: Element<'a, Message>,
    ) -> Element<'a, Message> {
        let mut column = widget::column::with_capacity(5)
            .push(
                widget::row::with_children(vec![
                    widget::text::title1(title).into(),
                    widget::space().width(Length::Fill).into(),
                    widget::text::caption(subtitle).into(),
                ])
                .align_y(Alignment::Center),
            )
            .spacing(12)
            .padding([8, 16]);

        if let Some(error) = &self.error {
            column = column.push(
                widget::container(widget::text::body(format!("No se pudo cargar: {error}")))
                    .class(cosmic::theme::Container::Card)
                    .padding(12)
                    .width(Length::Fill),
            );
        }
        if let Some(notice) = &self.notice {
            column = column.push(
                widget::container(widget::text::body(notice))
                    .class(cosmic::theme::Container::Card)
                    .padding(12)
                    .width(Length::Fill),
            );
        }

        column.push(content).height(Length::Fill).into()
    }

    fn comics_view(&self) -> Element<'_, Message> {
        let content = if self.loading && self.comics.is_empty() {
            self.loading_view("Cargando biblioteca…")
        } else if self.comics.is_empty() {
            self.empty_view(
                "folder-symbolic",
                "Sin cómics",
                "Añade una carpeta en Preferencias y ejecuta el escaneo desde la app principal.",
            )
        } else {
            widget::responsive(|size| {
                // Leave room for the scrollbar and the grid's horizontal padding.
                let available = (size.width - COMIC_GRID_SCROLLBAR_RESERVE).max(COMIC_CARD_WIDTH);
                let columns = ((available + COMIC_GRID_GAP) / (COMIC_CARD_WIDTH + COMIC_GRID_GAP))
                    .floor()
                    .max(1.0) as usize;
                // Spread the leftover width across the gaps so rows fill the page.
                let gap = if columns > 1 {
                    ((available - columns as f32 * COMIC_CARD_WIDTH) / (columns - 1) as f32)
                        .max(COMIC_GRID_GAP)
                } else {
                    COMIC_GRID_GAP
                };
                self.comic_grid(columns, gap)
            })
            .into()
        };
        self.page_shell(
            "Cómics",
            if self.selected_comics.is_empty() {
                format!("{} de {}", self.comics.len(), self.comics_total)
            } else {
                format!(
                    "{} de {} · {} seleccionados",
                    self.comics.len(),
                    self.comics_total,
                    self.selected_comics.len()
                )
            },
            content,
        )
    }

    fn comic_grid(&self, columns: usize, gap: f32) -> Element<'_, Message> {
        let mut rows =
            widget::column::with_capacity(self.comics.len().div_ceil(columns)).spacing(14);
        for chunk in self.comics.chunks(columns) {
            let mut row = widget::row::with_capacity(columns).spacing(gap);
            for comic in chunk {
                row = row.push(self.comic_card(comic));
            }
            for _ in chunk.len()..columns {
                row = row.push(widget::space().width(COMIC_CARD_WIDTH));
            }
            rows = rows.push(row);
        }
        widget::scrollable(rows.padding([4, 2]))
            .id(widget::Id::new(COMIC_GRID_SCROLL_ID))
            .on_scroll(move |viewport| {
                Message::ComicGridScrolled(ComicGridViewport {
                    offset_y: viewport.absolute_offset().y,
                    content_height: viewport.content_bounds().height,
                    columns,
                    context_shown: false,
                })
            })
            .height(Length::Fill)
            .into()
    }

    fn comic_card<'a>(&self, comic: &'a ComicbookView) -> Element<'a, Message> {
        let selected = self.selected_comics.contains(&comic.id_comicbook);
        let fallback_title = comic
            .titulo
            .as_deref()
            .or_else(|| Path::new(&comic.path).file_name().and_then(|v| v.to_str()))
            .unwrap_or("Cómic");
        let title = comic.nombre_volume.as_deref().unwrap_or(fallback_title);
        let number = comic.numero.as_deref().unwrap_or("—");
        let subtitle = match (&comic.nombre_volume, &comic.titulo) {
            (Some(_), Some(issue_title)) if issue_title != title => {
                format!("#{number} · {issue_title}")
            }
            _ if comic.titulo.is_some() => format!("#{number}"),
            _ => "Sin catalogar".to_string(),
        };
        let total_pages = comic.reading_total_pages.max(0);
        let visited_pages = comic.reading_visited_pages.clamp(0, total_pages);
        let completion = if total_pages > 0 {
            (visited_pages as f32 / total_pages as f32).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let has_progress = visited_pages > 0;
        let completed = total_pages > 0 && visited_pages == total_pages;
        let cover = comic_thumbnail_path(comic.id_comicbook, CardSize::Medium);
        let visual: Element<_> = if cover.exists() {
            widget::image(cosmic::iced::widget::image::Handle::from_path(cover))
                .width(COMIC_COVER_WIDTH)
                .height(COMIC_COVER_HEIGHT)
                .content_fit(cosmic::iced::ContentFit::Contain)
                .into()
        } else {
            widget::container(icon::from_name("image-missing-symbolic").size(48).icon())
                .width(COMIC_COVER_WIDTH)
                .height(COMIC_COVER_HEIGHT)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center)
                .class(cosmic::theme::Container::Secondary)
                .into()
        };
        let cover_with_status: Element<_> = if has_progress {
            let reading_status = widget::container(widget::text::caption_heading(if completed {
                "LEÍDO"
            } else {
                "LEYENDO"
            }))
            .class(if completed {
                cosmic::theme::Container::Primary
            } else {
                cosmic::theme::Container::Secondary
            })
            .padding([3, 6]);
            let percentage = widget::container(widget::text::caption_heading(format!(
                "{:.0}%",
                completion * 100.0
            )))
            .class(cosmic::theme::Container::Secondary)
            .padding([3, 6]);
            let overlay = widget::container(
                widget::row::with_children(vec![
                    reading_status.into(),
                    widget::space().width(Length::Fill).into(),
                    percentage.into(),
                ])
                .width(Length::Fill),
            )
            .width(COMIC_COVER_WIDTH)
            .height(COMIC_COVER_HEIGHT)
            .align_y(Vertical::Top)
            .padding(7);
            cosmic::iced::widget::Stack::with_children(vec![visual, overlay.into()])
                .width(COMIC_COVER_WIDTH)
                .height(COMIC_COVER_HEIGHT)
                .into()
        } else {
            visual
        };
        let title_row = widget::row::with_children(vec![
            widget::text::heading(title)
                .width(Length::Fill)
                .height(42)
                .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(2)))
                .into(),
            if selected {
                widget::tooltip(
                    icon::from_name("emblem-ok-symbolic").size(20).icon(),
                    widget::text("Seleccionado"),
                    widget::tooltip::Position::Top,
                )
                .into()
            } else {
                widget::space().width(0).into()
            },
        ])
        .height(42)
        .align_y(Alignment::Start)
        .spacing(6);
        let information = widget::column::with_children(vec![
            title_row.into(),
            widget::text::caption(subtitle)
                .height(17)
                .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)))
                .into(),
        ])
        .spacing(4);
        let main_content = widget::column::with_children(vec![
            cover_with_status,
            widget::progress_bar::linear::Linear::new()
                .progress(completion)
                .girth(4)
                .width(COMIC_COVER_WIDTH)
                .into(),
            widget::space().height(2).into(),
            information.into(),
        ])
        .spacing(6);
        // Note: `Button::Transparent` forces a fully transparent text color,
        // which would hide the title/subtitle. A mouse area keeps theme colors.
        // Un clic selecciona; el detalle se abre con doble clic o Ctrl+Espacio.
        let open_detail = widget::mouse_area(main_content.width(COMIC_COVER_WIDTH))
            .on_press(Message::ComicClicked(comic.id_comicbook))
            .on_double_click(Message::OpenComicDetail(comic.id_comicbook));
        let read_label = if has_progress && !completed {
            "Continuar"
        } else if completed {
            "Releer"
        } else {
            "Leer ahora"
        };
        let actions = widget::row::with_children(vec![
            widget::button::suggested(read_label)
                .leading_icon(icon::from_name("media-playback-start-symbolic"))
                .on_press(Message::ReadComic(comic.id_comicbook, comic.path.clone()))
                .width(Length::Fill)
                .into(),
            widget::tooltip(
                widget::button::icon(icon::from_name("view-more-symbolic"))
                    .on_press(Message::OpenComicDetail(comic.id_comicbook)),
                widget::text("Más opciones"),
                widget::tooltip::Position::Top,
            )
            .into(),
        ])
        .spacing(6)
        .align_y(Alignment::Center);
        widget::container(
            widget::column::with_children(vec![open_detail.into(), actions.into()]).spacing(9),
        )
        .class(if selected {
            cosmic::theme::Container::Primary
        } else {
            cosmic::theme::Container::Card
        })
        .padding(12)
        .width(COMIC_CARD_WIDTH)
        .into()
    }

    fn volumes_view(&self) -> Element<'_, Message> {
        let content = if self.loading && self.volumes.is_empty() {
            self.loading_view("Cargando series…")
        } else if self.volumes.is_empty() {
            self.empty_view(
                "view-grid-symbolic",
                "Sin series",
                "Las series aparecerán al catalogar cómics.",
            )
        } else {
            widget::responsive(|size| {
                let columns = ((size.width / 240.0).floor() as usize).clamp(2, 6);
                self.volume_grid(columns)
            })
            .into()
        };
        let subtitle = if self.selected_volumes.is_empty() {
            format!("{} series", self.volumes.len())
        } else {
            format!(
                "{} series · {} seleccionadas",
                self.volumes.len(),
                self.selected_volumes.len()
            )
        };
        self.page_shell("Series", subtitle, content)
    }

    fn volume_grid(&self, columns: usize) -> Element<'_, Message> {
        let mut rows =
            widget::column::with_capacity(self.volumes.len().div_ceil(columns)).spacing(14);
        for chunk in self.volumes.chunks(columns) {
            let mut row = widget::row::with_capacity(columns).spacing(14);
            for item in chunk {
                let volume = &item.volume;
                let selected = self.selected_volumes.contains(&volume.id_volume);
                let cover = volume_thumbnail_path(volume.id_volume);
                let visual: Element<_> = if cover.exists() {
                    widget::image(cosmic::iced::widget::image::Handle::from_path(cover))
                        .height(180)
                        .width(Length::Fill)
                        .content_fit(cosmic::iced::ContentFit::Contain)
                        .into()
                } else {
                    widget::container(icon::from_name("image-missing-symbolic").size(40).icon())
                        .height(180)
                        .width(Length::Fill)
                        .align_x(Horizontal::Center)
                        .align_y(Vertical::Center)
                        .class(cosmic::theme::Container::Secondary)
                        .into()
                };
                let progress = format!(
                    "{} / {} números · {}",
                    volume.cantidad_poseida, volume.cantidad_numeros, volume.anio_inicio
                );
                let update_status = if item.has_api_updates {
                    "Con actualizaciones de ComicVine"
                } else {
                    "Sin actualizaciones de ComicVine"
                };
                let mut card = widget::column::with_children(vec![
                    visual,
                    widget::text::body(&volume.nombre).into(),
                    widget::text::caption(progress).into(),
                    widget::text::caption(update_status).into(),
                ])
                .spacing(6);
                if selected {
                    card = card.push(widget::text::caption("Seleccionado"));
                }
                row = row.push(
                    widget::button::custom(
                        widget::container(card)
                            .class(if selected {
                                cosmic::theme::Container::Primary
                            } else {
                                cosmic::theme::Container::Card
                            })
                            .padding(8)
                            .width(Length::Fill),
                    )
                    .on_press(Message::VolumeClicked(volume.id_volume))
                    .selected(selected)
                    .class(cosmic::theme::Button::Transparent)
                    .padding(0)
                    .width(Length::FillPortion(1)),
                );
            }
            for _ in chunk.len()..columns {
                row = row.push(widget::space().width(Length::Fill));
            }
            rows = rows.push(row);
        }
        widget::scrollable(rows.padding([4, 2]))
            .height(Length::Fill)
            .into()
    }

    fn publishers_view(&self) -> Element<'_, Message> {
        let content = if self.loading && self.publishers.is_empty() {
            self.loading_view("Cargando editoriales…")
        } else if self.publishers.is_empty() {
            self.empty_view(
                "avatar-default-symbolic",
                "Sin editoriales",
                "Las editoriales aparecerán al descargar metadatos.",
            )
        } else {
            let mut list = widget::column::with_capacity(self.publishers.len()).spacing(8);
            for publisher in &self.publishers {
                let subtitle = publisher
                    .descripcion
                    .as_deref()
                    .filter(|v| !v.is_empty())
                    .unwrap_or("Editorial de la biblioteca");
                let item = widget::row::with_children(vec![
                    icon::from_name("avatar-default-symbolic")
                        .size(28)
                        .icon()
                        .into(),
                    widget::column::with_children(vec![
                        widget::text::body(&publisher.nombre).into(),
                        widget::text::caption(subtitle).into(),
                    ])
                    .spacing(2)
                    .width(Length::Fill)
                    .into(),
                    icon::from_name("go-next-symbolic").size(16).icon().into(),
                ])
                .spacing(12)
                .align_y(Alignment::Center);
                list = list.push(
                    widget::button::custom(
                        widget::container(item)
                            .class(cosmic::theme::Container::Card)
                            .padding(12)
                            .width(Length::Fill),
                    )
                    .on_press(Message::OpenDrawer(Box::new(Drawer::Publisher(
                        publisher.clone(),
                    ))))
                    .class(cosmic::theme::Button::Transparent)
                    .padding(0)
                    .width(Length::Fill),
                );
            }
            widget::scrollable(list.padding([4, 2]))
                .height(Length::Fill)
                .into()
        };
        self.page_shell(
            "Editoriales",
            format!("{} editoriales", self.publishers.len()),
            content,
        )
    }

    fn comicvine_view(&self) -> Element<'_, Message> {
        let volume_mode = if self.comicvine_search_mode == ComicVineSearchMode::Volumes {
            widget::button::suggested("Volúmenes")
        } else {
            widget::button::standard("Volúmenes")
        }
        .on_press_maybe(
            (!self.comicvine_searching && !self.comicvine_importing).then_some(
                Message::SetComicVineSearchMode(ComicVineSearchMode::Volumes),
            ),
        )
        .width(Length::FillPortion(1));
        let publisher_mode = if self.comicvine_search_mode == ComicVineSearchMode::Publishers {
            widget::button::suggested("Editoriales")
        } else {
            widget::button::standard("Editoriales")
        }
        .on_press_maybe(
            (!self.comicvine_searching && !self.comicvine_importing).then_some(
                Message::SetComicVineSearchMode(ComicVineSearchMode::Publishers),
            ),
        )
        .width(Length::FillPortion(1));
        let mode_selector =
            widget::row::with_children(vec![volume_mode.into(), publisher_mode.into()]).spacing(8);

        let placeholder = match self.comicvine_search_mode {
            ComicVineSearchMode::Volumes => "Nombre del volumen…",
            ComicVineSearchMode::Publishers => "Nombre de la editorial…",
        };
        let search = widget::text_input(placeholder, &self.comicvine_query)
            .on_input(Message::ComicVineQueryChanged)
            .on_submit(|_| Message::SearchComicVine)
            .width(Length::Fill);
        let search_button = widget::button::suggested(if self.comicvine_searching {
            "Buscando…"
        } else {
            "Buscar"
        })
        .leading_icon(icon::from_name("system-search-symbolic"))
        .on_press_maybe(
            (!self.comicvine_searching && !self.comicvine_importing)
                .then_some(Message::SearchComicVine),
        );
        let controls = widget::row::with_children(vec![search.into(), search_button.into()])
            .spacing(8)
            .align_y(Alignment::Center);
        let queue_label = if self.comicvine_selected.is_empty() {
            "Selecciona volúmenes".to_string()
        } else {
            format!("Agregar {} a la cola", self.comicvine_selected.len())
        };

        let action_controls: Element<_> = match self.comicvine_search_mode {
            ComicVineSearchMode::Volumes => widget::container(
                widget::row::with_children(vec![
                    widget::checkbox(self.comicvine_download_covers)
                        .label("Descargar también las portadas")
                        .on_toggle(Message::ToggleDownloadCovers)
                        .width(Length::Fill)
                        .into(),
                    widget::button::suggested(queue_label)
                        .leading_icon(icon::from_name("folder-download-symbolic"))
                        .on_press_maybe(
                            (!self.comicvine_selected.is_empty())
                                .then_some(Message::QueueComicVineDownloads),
                        )
                        .into(),
                ])
                .spacing(12)
                .align_y(Alignment::Center),
            )
            .class(cosmic::theme::Container::Card)
            .padding(12)
            .width(Length::Fill)
            .into(),
            ComicVineSearchMode::Publishers => widget::container(
                widget::row::with_children(vec![
                    widget::text::body(
                        "Importa la editorial y todos sus volúmenes sin duplicar los existentes.",
                    )
                    .width(Length::Fill)
                    .into(),
                    widget::button::suggested(if self.comicvine_importing {
                        "Importando…"
                    } else if self.comicvine_publishers_selected.is_empty() {
                        "Selecciona editoriales"
                    } else {
                        "Importar seleccionadas"
                    })
                    .leading_icon(icon::from_name("folder-download-symbolic"))
                    .on_press_maybe(
                        (!self.comicvine_importing
                            && !self.comicvine_publishers_selected.is_empty())
                        .then_some(Message::ImportComicVinePublishers),
                    )
                    .into(),
                ])
                .spacing(12)
                .align_y(Alignment::Center),
            )
            .class(cosmic::theme::Container::Card)
            .padding(12)
            .width(Length::Fill)
            .into(),
        };

        let results_empty = match self.comicvine_search_mode {
            ComicVineSearchMode::Volumes => self.comicvine_results.is_empty(),
            ComicVineSearchMode::Publishers => self.comicvine_publisher_results.is_empty(),
        };

        let results: Element<_> = if self.comicvine_searching {
            self.loading_view("Consultando ComicVine…")
        } else if let Some(error) = &self.comicvine_error {
            self.empty_view("dialog-error-symbolic", "No se pudo buscar", error)
        } else if results_empty && self.comicvine_has_searched {
            self.empty_view(
                "edit-find-symbolic",
                "Sin resultados",
                match self.comicvine_search_mode {
                    ComicVineSearchMode::Volumes => {
                        "ComicVine no devolvió volúmenes para esa búsqueda."
                    }
                    ComicVineSearchMode::Publishers => {
                        "ComicVine no devolvió editoriales para esa búsqueda."
                    }
                },
            )
        } else if results_empty {
            self.empty_view(
                "system-search-symbolic",
                match self.comicvine_search_mode {
                    ComicVineSearchMode::Volumes => "Buscar volúmenes",
                    ComicVineSearchMode::Publishers => "Buscar editoriales",
                },
                match self.comicvine_search_mode {
                    ComicVineSearchMode::Volumes => {
                        "Escribe el nombre de un volumen para consultar ComicVine."
                    }
                    ComicVineSearchMode::Publishers => {
                        "Escribe el nombre de una editorial para importar su catálogo."
                    }
                },
            )
        } else {
            widget::responsive(|size| {
                let columns = ((size.width / 300.0).floor() as usize).clamp(1, 4);
                let item_count = match self.comicvine_search_mode {
                    ComicVineSearchMode::Volumes => self.comicvine_results.len(),
                    ComicVineSearchMode::Publishers => self.comicvine_publisher_results.len(),
                };
                let mut rows =
                    widget::column::with_capacity(item_count.div_ceil(columns)).spacing(12);
                for offset in (0..item_count).step_by(columns) {
                    let mut row = widget::row::with_capacity(columns).spacing(12);
                    let end = (offset + columns).min(item_count);
                    match self.comicvine_search_mode {
                        ComicVineSearchMode::Volumes => {
                            for volume in &self.comicvine_results[offset..end] {
                                row = row.push(self.comicvine_volume_card(volume));
                            }
                        }
                        ComicVineSearchMode::Publishers => {
                            for publisher in &self.comicvine_publisher_results[offset..end] {
                                row = row.push(self.comicvine_publisher_card(publisher));
                            }
                        }
                    }
                    for _ in (end - offset)..columns {
                        row = row.push(widget::space().width(Length::Fill));
                    }
                    rows = rows.push(row);
                }
                widget::scrollable(rows.padding([4, 2]))
                    .height(Length::Fill)
                    .into()
            })
            .into()
        };

        let content = widget::column::with_children(vec![
            mode_selector.into(),
            controls.into(),
            action_controls,
            results,
        ])
        .spacing(10)
        .height(Length::Fill)
        .into();
        let (result_count, selected_count) = match self.comicvine_search_mode {
            ComicVineSearchMode::Volumes => {
                (self.comicvine_results.len(), self.comicvine_selected.len())
            }
            ComicVineSearchMode::Publishers => (
                self.comicvine_publisher_results.len(),
                self.comicvine_publishers_selected.len(),
            ),
        };
        self.page_shell(
            "ComicVine",
            format!("{result_count} resultados · {selected_count} seleccionados"),
            content,
        )
    }

    fn comicvine_volume_card<'a>(&self, volume: &'a CvVolumeResult) -> Element<'a, Message> {
        let selected = self.comicvine_selected.contains(&volume.cv_id);
        let download_status = DownloadManager::get_instance(self.pool.clone())
            .get_state()
            .lock()
            .unwrap()
            .get(&volume.cv_id.to_string())
            .map(|download| download.status.clone());
        let is_queued_or_downloading = matches!(
            &download_status,
            Some(DownloadStatus::Queued | DownloadStatus::Downloading)
        );
        let status_text = match &download_status {
            Some(DownloadStatus::Queued) => Some("En cola"),
            Some(DownloadStatus::Downloading) => Some("Descargando"),
            Some(DownloadStatus::Completed) => Some("Descargado"),
            Some(DownloadStatus::Error(_)) => Some("Error; puedes reintentarlo"),
            None => None,
        };
        let visual = widget::container(
            icon::from_name(if is_queued_or_downloading {
                "folder-download-symbolic"
            } else if selected {
                "emblem-ok-symbolic"
            } else {
                "image-x-generic-symbolic"
            })
            .size(48)
            .icon(),
        )
        .height(150)
        .width(Length::Fill)
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .class(cosmic::theme::Container::Secondary);
        let mut content = widget::column::with_children(vec![
            visual.into(),
            widget::text::title2(&volume.name).into(),
            widget::text::body(&volume.publisher_name).into(),
            widget::text::caption(format!(
                "{} · {} números · ID {}",
                volume.start_year, volume.count_of_issues, volume.cv_id
            ))
            .into(),
        ])
        .spacing(5);
        if let Some(status) = status_text {
            content = content.push(widget::text::caption(status));
        }
        widget::button::custom(
            widget::container(content)
                .class(if selected {
                    cosmic::theme::Container::Primary
                } else {
                    cosmic::theme::Container::Card
                })
                .padding(10)
                .width(Length::Fill),
        )
        .on_press_maybe(
            (!is_queued_or_downloading).then_some(Message::ToggleComicVineSelection(volume.cv_id)),
        )
        .selected(selected)
        .class(cosmic::theme::Button::Transparent)
        .padding(0)
        .width(Length::FillPortion(1))
        .into()
    }

    fn comicvine_publisher_card<'a>(
        &self,
        publisher: &'a CvPublisherResult,
    ) -> Element<'a, Message> {
        let selected = self
            .comicvine_publishers_selected
            .contains(&publisher.cv_id);
        let visual = widget::container(
            icon::from_name(if selected {
                "emblem-ok-symbolic"
            } else {
                "avatar-default-symbolic"
            })
            .size(48)
            .icon(),
        )
        .height(150)
        .width(Length::Fill)
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .class(cosmic::theme::Container::Secondary);
        let location = if publisher.location_city.is_empty() {
            "Ubicación desconocida"
        } else {
            publisher.location_city.as_str()
        };
        let content = widget::column::with_children(vec![
            visual.into(),
            widget::text::title2(&publisher.name).into(),
            widget::text::body(location).into(),
            widget::text::caption(format!(
                "{} números acreditados · ID {}",
                publisher.count_of_issues, publisher.cv_id
            ))
            .into(),
        ])
        .spacing(5);
        widget::button::custom(
            widget::container(content)
                .class(if selected {
                    cosmic::theme::Container::Primary
                } else {
                    cosmic::theme::Container::Card
                })
                .padding(10)
                .width(Length::Fill),
        )
        .on_press(Message::ToggleComicVinePublisherSelection(publisher.cv_id))
        .selected(selected)
        .class(cosmic::theme::Button::Transparent)
        .padding(0)
        .width(Length::FillPortion(1))
        .into()
    }

    fn downloads_view(&self) -> Element<'_, Message> {
        let content = if self.jobs.is_empty() {
            self.empty_view(
                "folder-download-symbolic",
                "Sin actividad",
                "Las descargas, los escaneos y la indexación aparecerán aquí.",
            )
        } else {
            let mut list = widget::column::with_capacity(self.jobs.len()).spacing(8);
            for job in &self.jobs {
                let (icon_name, status) = match job.status {
                    BackgroundJobStatus::Running
                        if job.kind == JobKind::Download && job.message == "En cola..." =>
                    {
                        ("folder-download-symbolic", "En cola")
                    }
                    BackgroundJobStatus::Running => ("content-loading-symbolic", "En curso"),
                    BackgroundJobStatus::Completed => ("emblem-ok-symbolic", "Completado"),
                    BackgroundJobStatus::Error => ("dialog-error-symbolic", "Error"),
                };
                let row = widget::row::with_children(vec![
                    icon::from_name(icon_name).size(24).icon().into(),
                    widget::column::with_children(vec![
                        widget::text::body(&job.title).into(),
                        widget::text::caption(&job.message).into(),
                    ])
                    .spacing(2)
                    .width(Length::Fill)
                    .into(),
                    widget::text::caption(format!("{status} · {:.0}%", job.progress * 100.0))
                        .into(),
                ])
                .spacing(12)
                .align_y(Alignment::Center);
                list = list.push(
                    widget::container(row)
                        .class(cosmic::theme::Container::Card)
                        .padding(14)
                        .width(Length::Fill),
                );
            }
            widget::scrollable(list.padding([4, 2]))
                .height(Length::Fill)
                .into()
        };

        let shell = widget::column::with_capacity(3)
            .push(
                widget::row::with_children(vec![
                    widget::text::title1("Actividad").into(),
                    widget::space().width(Length::Fill).into(),
                    widget::text::caption(format!("{} tareas", self.jobs.len())).into(),
                ])
                .align_y(Alignment::Center),
            )
            .push(content)
            .spacing(12)
            .padding([8, 16])
            .height(Length::Fill);
        shell.into()
    }

    fn loading_view<'a>(&self, label: &'a str) -> Element<'a, Message> {
        widget::container(
            widget::column::with_children(vec![
                widget::icon::from_name("content-loading-symbolic")
                    .size(48)
                    .icon()
                    .into(),
                widget::text::body(label).into(),
            ])
            .spacing(12)
            .align_x(Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .into()
    }

    fn empty_view<'a>(
        &self,
        icon_name: &'static str,
        title: &'a str,
        description: &'a str,
    ) -> Element<'a, Message> {
        widget::container(
            widget::column::with_children(vec![
                icon::from_name(icon_name).size(64).icon().into(),
                widget::text::title2(title).into(),
                widget::text::body(description).into(),
            ])
            .spacing(10)
            .align_x(Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .into()
    }

    fn about_view(&self) -> Element<'_, Message> {
        widget::column::with_children(vec![
            icon::from_name("accessories-text-editor-symbolic").size(72).icon().into(),
            widget::text::title1("Babelcomics").into(),
            widget::text::body("Organizador y catalogador de cómics").into(),
            widget::text::caption(format!("Versión {} · interfaz libcosmic", env!("CARGO_PKG_VERSION"))).into(),
            widget::text::body("La biblioteca sigue usando la misma base SQLite y el mismo núcleo que la interfaz GTK.").into(),
        ])
        .spacing(12)
        .padding(16)
        .align_x(Alignment::Center)
        .into()
    }

    fn search_filters_view(&self) -> Element<'_, Message> {
        let filter_button = |label: &'static str, filter: CatalogFilter| {
            let button = if self.catalog_filter == filter {
                widget::button::suggested(label)
            } else {
                widget::button::standard(label)
            };
            button
                .on_press(Message::SetCatalogFilter(filter))
                .width(Length::Fill)
        };
        let mut content = widget::column::with_children(vec![
            widget::text::title2("Estado de catalogación").into(),
            widget::text::body(
                "Este filtro se combina con el texto de búsqueda y también puede cambiarse en Configuración > General.",
            )
            .into(),
            filter_button("Todos", CatalogFilter::All).into(),
            filter_button("Catalogados", CatalogFilter::Catalogued).into(),
            filter_button("Sin catalogar", CatalogFilter::Uncatalogued).into(),
        ])
        .spacing(8);
        if self.nav.active_data::<Page>() == Some(&Page::Volumes) {
            let update_button = |label: &'static str, filter: VolumeUpdateFilter| {
                let button = if self.volume_update_filter == filter {
                    widget::button::suggested(label)
                } else {
                    widget::button::standard(label)
                };
                button
                    .on_press(Message::SetVolumeUpdateFilter(filter))
                    .width(Length::Fill)
            };
            content = content
                .push(widget::text::title2("Actualización desde ComicVine"))
                .push(update_button("Ambos", VolumeUpdateFilter::All))
                .push(update_button(
                    "Con actualizaciones",
                    VolumeUpdateFilter::Updated,
                ))
                .push(update_button(
                    "Sin actualizaciones",
                    VolumeUpdateFilter::NotUpdated,
                ));

            let sort_row = |label: &'static str, asc: VolumeSortOrder, desc: VolumeSortOrder| {
                widget::row::with_children(vec![
                    widget::text::body(label).width(Length::Fill).into(),
                    widget::button::text("Asc")
                        .on_press_maybe(
                            (self.volume_sort_order != asc)
                                .then_some(Message::SetVolumeSortOrder(asc)),
                        )
                        .into(),
                    widget::button::text("Desc")
                        .on_press_maybe(
                            (self.volume_sort_order != desc)
                                .then_some(Message::SetVolumeSortOrder(desc)),
                        )
                        .into(),
                ])
                .spacing(8)
                .align_y(Alignment::Center)
            };
            content = content
                .push(widget::text::title2("Ordenar series"))
                .push(sort_row(
                    "Nombre",
                    VolumeSortOrder::NombreAsc,
                    VolumeSortOrder::NombreDesc,
                ))
                .push(sort_row(
                    "Año",
                    VolumeSortOrder::AnioAsc,
                    VolumeSortOrder::AnioDesc,
                ))
                .push(sort_row(
                    "Números",
                    VolumeSortOrder::IssuesAsc,
                    VolumeSortOrder::IssuesDesc,
                ))
                .push(sort_row(
                    "Completitud",
                    VolumeSortOrder::CompletitudAsc,
                    VolumeSortOrder::CompletitudDesc,
                ));
        }
        content.padding(12).into()
    }

    fn preferences_view(&self) -> Element<'_, Message> {
        let page = match self.preferences_page {
            PreferencesPage::Menu => self.preferences_menu_view(),
            PreferencesPage::General => self.general_preferences_view(),
            PreferencesPage::ComicVine => self.comicvine_preferences_view(),
            PreferencesPage::ScanFolders => self.scan_folders_preferences_view(),
            PreferencesPage::NormalizeCatalog => self.normalize_catalog_preferences_view(),
            PreferencesPage::Reader => self.reader_preferences_view(),
            PreferencesPage::Advanced => self.advanced_preferences_view(),
        };

        widget::scrollable(page)
            .height(Length::Fill)
            .padding(12)
            .into()
    }

    fn preferences_back_button(&self) -> Element<'_, Message> {
        widget::button::text("Volver a configuración")
            .leading_icon(icon::from_name("go-previous-symbolic"))
            .on_press(Message::SelectPreferencesPage(PreferencesPage::Menu))
            .into()
    }

    fn preferences_menu_view(&self) -> Element<'_, Message> {
        let option = |title: &'static str,
                      description: &'static str,
                      icon_name: &'static str,
                      page: PreferencesPage| {
            widget::button::custom(
                widget::container(
                    widget::row::with_children(vec![
                        icon::from_name(icon_name).size(28).icon().into(),
                        widget::column::with_children(vec![
                            widget::text::body(title).into(),
                            widget::text::caption(description).into(),
                        ])
                        .spacing(3)
                        .width(Length::Fill)
                        .into(),
                        icon::from_name("go-next-symbolic").size(16).icon().into(),
                    ])
                    .spacing(12)
                    .align_y(Alignment::Center),
                )
                .class(cosmic::theme::Container::Card)
                .padding(12)
                .width(Length::Fill),
            )
            .class(cosmic::theme::Button::Transparent)
            .padding(0)
            .width(Length::Fill)
            .on_press(Message::SelectPreferencesPage(page))
        };
        widget::column::with_children(vec![
            widget::text::title2("Configuración general").into(),
            widget::text::body("Elige qué parte de Babelcomics quieres configurar.").into(),
            option(
                "General",
                "Filtros predeterminados de la biblioteca",
                "preferences-system-symbolic",
                PreferencesPage::General,
            )
            .into(),
            option(
                "API ComicVine",
                "Clave, servidor y prueba de conexión",
                "network-transmit-receive-symbolic",
                PreferencesPage::ComicVine,
            )
            .into(),
            option(
                "Carpetas a escanear",
                "Orígenes, tipos de archivo y escaneo",
                "folder-open-symbolic",
                PreferencesPage::ScanFolders,
            )
            .into(),
            option(
                "Normalizar catálogo",
                "Destino y organización de archivos",
                "folder-new-symbolic",
                PreferencesPage::NormalizeCatalog,
            )
            .into(),
            option(
                "Lector",
                "Ejecutable, ajuste de página, sentido y calidad",
                "x-office-document-symbolic",
                PreferencesPage::Reader,
            )
            .into(),
            option(
                "Avanzado",
                "Miniaturas, rendimiento e índice visual",
                "applications-engineering-symbolic",
                PreferencesPage::Advanced,
            )
            .into(),
        ])
        .spacing(10)
        .into()
    }

    fn general_preferences_view(&self) -> Element<'_, Message> {
        let catalog_button = |label: &'static str, filter: CatalogFilter| {
            let button = if self.catalog_filter == filter {
                widget::button::suggested(label)
            } else {
                widget::button::standard(label)
            };
            button
                .on_press(Message::SetCatalogFilter(filter))
                .width(Length::Fill)
        };
        let update_button = |label: &'static str, filter: VolumeUpdateFilter| {
            let button = if self.volume_update_filter == filter {
                widget::button::suggested(label)
            } else {
                widget::button::standard(label)
            };
            button
                .on_press(Message::SetVolumeUpdateFilter(filter))
                .width(Length::Fill)
        };
        widget::column::with_children(vec![
            self.preferences_back_button(),
            widget::text::title2("Filtros de biblioteca").into(),
            widget::text::body("Estas opciones se conservan y se aplican tanto a Cómics como a Series.").into(),
            widget::text::heading("Estado de catalogación").into(),
            catalog_button("Todos", CatalogFilter::All).into(),
            catalog_button("Solo catalogados", CatalogFilter::Catalogued).into(),
            catalog_button("Solo sin catalogar", CatalogFilter::Uncatalogued).into(),
            widget::text::heading("Actualización de series").into(),
            widget::text::caption("Una serie está actualizada cuando ya descargó su listado de números desde ComicVine.").into(),
            update_button("Con y sin actualizaciones", VolumeUpdateFilter::All).into(),
            update_button("Con actualizaciones", VolumeUpdateFilter::Updated).into(),
            update_button("Sin actualizaciones", VolumeUpdateFilter::NotUpdated).into(),
        ])
        .spacing(8)
        .into()
    }

    fn comicvine_preferences_view(&self) -> Element<'_, Message> {
        let api_key = widget::text_input("API key de ComicVine", &self.api_key_input)
            .password()
            .on_input(Message::ApiKeyChanged)
            .width(Length::Fill);
        let api_url = widget::text_input("URL de la API", &self.api_url_input)
            .on_input(Message::ApiUrlChanged)
            .width(Length::Fill);
        let mut content = widget::column::with_children(vec![
            self.preferences_back_button(),
            widget::text::title2("API ComicVine").into(),
            widget::text::body(
                "Configura el acceso utilizado para buscar y descargar metadatos y portadas.",
            )
            .into(),
            api_key.into(),
            api_url.into(),
            widget::row::with_children(vec![
                widget::button::standard(if self.api_testing {
                    "Probando…"
                } else {
                    "Probar conexión"
                })
                .leading_icon(icon::from_name("network-transmit-receive-symbolic"))
                .on_press_maybe((!self.api_testing).then_some(Message::TestComicVineConnection))
                .into(),
                widget::button::suggested(if self.api_saving {
                    "Guardando…"
                } else {
                    "Guardar"
                })
                .on_press_maybe((!self.api_saving).then_some(Message::SaveApiSettings))
                .into(),
            ])
            .spacing(8)
            .into(),
        ])
        .spacing(8);
        if let Some(status) = &self.api_status {
            content = content.push(
                widget::container(widget::text::body(status))
                    .class(cosmic::theme::Container::Card)
                    .padding(10)
                    .width(Length::Fill),
            );
        }
        content.into()
    }

    fn scan_folders_preferences_view(&self) -> Element<'_, Message> {
        let directory_input = widget::text_input("/ruta/a/mis/comics", &self.directory_input)
            .on_input(Message::DirectoryInputChanged)
            .on_submit(|_| Message::AddDirectory)
            .width(Length::Fill);
        let mut content = widget::column::with_children(vec![
            self.preferences_back_button(),
            widget::text::title2("Carpetas a escanear").into(),
            widget::text::body("Babelcomics buscará cómics dentro de todas estas carpetas.").into(),
            widget::row::with_children(vec![
                directory_input.into(),
                widget::button::text("Agregar")
                    .on_press_maybe((!self.directory_saving).then_some(Message::AddDirectory))
                    .into(),
            ])
            .spacing(8)
            .align_y(Alignment::Center)
            .into(),
            widget::button::standard("Elegir carpetas")
                .leading_icon(icon::from_name("folder-open-symbolic"))
                .on_press_maybe((!self.directory_saving).then_some(Message::ChooseDirectories))
                .width(Length::Fill)
                .into(),
        ])
        .spacing(8);
        if let Some(error) = &self.directory_error {
            content = content.push(widget::text::body(error));
        }
        if self.stats.directories.is_empty() {
            content = content.push(widget::text::caption("No hay carpetas configuradas."));
        } else {
            for directory in &self.stats.directories {
                content = content.push(
                    widget::container(
                        widget::row::with_children(vec![
                            icon::from_name("folder-symbolic").size(20).icon().into(),
                            widget::text::body(&directory.path)
                                .width(Length::Fill)
                                .into(),
                            widget::button::icon(icon::from_name("user-trash-symbolic"))
                                .on_press_maybe(
                                    (!self.directory_saving)
                                        .then_some(Message::RemoveDirectory(directory.id)),
                                )
                                .class(cosmic::theme::Button::Destructive)
                                .into(),
                        ])
                        .spacing(10)
                        .align_y(Alignment::Center),
                    )
                    .class(cosmic::theme::Container::Card)
                    .padding(10)
                    .width(Length::Fill),
                );
            }
        }
        let extension_checkbox = |label: &'static str, extension: &'static str| {
            widget::checkbox(self.stats.scan_extensions.contains(extension))
                .label(label)
                .on_toggle(move |enabled| Message::SetScanExtension(extension.to_string(), enabled))
                .width(Length::FillPortion(1))
        };
        content = content
            .push(widget::text::heading("Tipos de archivo"))
            .push(
                widget::row::with_children(vec![
                    extension_checkbox("CBZ", "cbz").into(),
                    extension_checkbox("CBR", "cbr").into(),
                    extension_checkbox("CB7", "cb7").into(),
                    extension_checkbox("PDF", "pdf").into(),
                ])
                .spacing(8),
            )
            .push(
                widget::row::with_children(vec![
                    extension_checkbox("ZIP", "zip").into(),
                    extension_checkbox("RAR", "rar").into(),
                    extension_checkbox("7Z", "7z").into(),
                    widget::space().width(Length::FillPortion(1)).into(),
                ])
                .spacing(8),
            )
            .push(
                widget::button::suggested(if self.scanning {
                    "Escaneando…"
                } else {
                    "Escanear carpetas"
                })
                .leading_icon(icon::from_name("system-run-symbolic"))
                .on_press_maybe((!self.scanning).then_some(Message::StartScan))
                .width(Length::Fill),
            );
        if let Some(status) = &self.scan_status {
            content = content.push(widget::text::caption(status));
        }
        content.into()
    }

    fn normalize_catalog_preferences_view(&self) -> Element<'_, Message> {
        let directory = self
            .stats
            .normalized_catalog_directory
            .as_deref()
            .unwrap_or("Sin configurar");
        let mut content = widget::column::with_children(vec![
            self.preferences_back_button(),
            widget::text::title2("Normalizar catálogo").into(),
            widget::text::body(
                "Mueve los cómics catalogados a Editorial/Volumen-Año/Volumen NNN.ext.",
            )
            .into(),
            self.info_row("Carpeta de destino", directory),
            widget::button::standard("Elegir carpeta del catálogo")
                .leading_icon(icon::from_name("folder-open-symbolic"))
                .on_press_maybe(
                    (!self.preference_saving && !self.organization_preparing && !self.organizing)
                        .then_some(Message::ChooseNormalizedCatalogDirectory),
                )
                .width(Length::Fill)
                .into(),
            widget::button::suggested(if self.organization_preparing {
                "Analizando biblioteca…"
            } else if self.organizing {
                "Organizando catálogo…"
            } else {
                "Preparar organización"
            })
            .leading_icon(icon::from_name("folder-new-symbolic"))
            .on_press_maybe(
                (self.stats.normalized_catalog_directory.is_some()
                    && !self.organization_preparing
                    && !self.organizing)
                    .then_some(Message::PrepareOrganization),
            )
            .width(Length::Fill)
            .into(),
        ])
        .spacing(8);
        if !self.organization_plan.is_empty() {
            let movable = self
                .organization_plan
                .iter()
                .filter(|plan| {
                    matches!(
                        plan.status,
                        OrganizationStatus::Ok | OrganizationStatus::Duplicate
                    )
                })
                .count();
            let already = self
                .organization_plan
                .iter()
                .filter(|plan| plan.status == OrganizationStatus::AlreadyInPlace)
                .count();
            let uncatalogued = self
                .organization_plan
                .iter()
                .filter(|plan| plan.status == OrganizationStatus::NoInfo)
                .count();
            let errors = self
                .organization_plan
                .iter()
                .filter(|plan| plan.status == OrganizationStatus::Error)
                .count();
            content = content.push(
                widget::container(
                    widget::column::with_children(vec![
                        widget::text::heading("Plan listo para confirmar").into(),
                        self.detail_value_row("Se moverán", movable.to_string()),
                        self.detail_value_row("Ya organizados", already.to_string()),
                        self.detail_value_row("Sin catalogar", uncatalogued.to_string()),
                        self.detail_value_row("Con errores", errors.to_string()),
                        widget::row::with_children(vec![
                            widget::button::standard("Cancelar")
                                .on_press_maybe(
                                    (!self.organizing).then_some(Message::CancelOrganization),
                                )
                                .width(Length::FillPortion(1))
                                .into(),
                            widget::button::suggested(if self.organizing {
                                "Moviendo…"
                            } else {
                                "Mover y organizar"
                            })
                            .on_press_maybe(
                                (movable > 0 && !self.organizing)
                                    .then_some(Message::ConfirmOrganization),
                            )
                            .width(Length::FillPortion(2))
                            .into(),
                        ])
                        .spacing(8)
                        .into(),
                    ])
                    .spacing(8),
                )
                .class(cosmic::theme::Container::Card)
                .padding(12)
                .width(Length::Fill),
            );
        }
        if let Some(error) = &self.preference_error {
            content = content.push(widget::text::body(error));
        }
        content.into()
    }

    #[allow(dead_code)]
    fn basic_preferences_view(&self) -> Element<'_, Message> {
        let api_key = widget::text_input("API key de ComicVine", &self.api_key_input)
            .password()
            .on_input(Message::ApiKeyChanged)
            .width(Length::Fill);
        let api_url = widget::text_input("URL de la API", &self.api_url_input)
            .on_input(Message::ApiUrlChanged)
            .width(Length::Fill);
        let save_api = widget::button::suggested(if self.api_saving {
            "Guardando…"
        } else {
            "Guardar"
        })
        .on_press_maybe((!self.api_saving).then_some(Message::SaveApiSettings));
        let test_api = widget::button::standard(if self.api_testing {
            "Probando…"
        } else {
            "Probar conexión"
        })
        .leading_icon(icon::from_name("network-transmit-receive-symbolic"))
        .on_press_maybe((!self.api_testing).then_some(Message::TestComicVineConnection));
        let directory_input = widget::text_input("/ruta/a/mis/comics", &self.directory_input)
            .on_input(Message::DirectoryInputChanged)
            .on_submit(|_| Message::AddDirectory)
            .width(Length::Fill);
        let add_button = widget::button::text("Agregar")
            .on_press_maybe((!self.directory_saving).then_some(Message::AddDirectory));
        let choose_button = widget::button::standard("Elegir carpetas")
            .leading_icon(icon::from_name("folder-open-symbolic"))
            .on_press_maybe((!self.directory_saving).then_some(Message::ChooseDirectories));
        let scan_button = widget::button::suggested(if self.scanning {
            "Escaneando…"
        } else {
            "Escanear carpetas"
        })
        .leading_icon(icon::from_name("system-run-symbolic"))
        .on_press_maybe((!self.scanning).then_some(Message::StartScan));

        let mut content = widget::column::with_capacity(14)
            .push(widget::text::title2("ComicVine API"))
            .push(widget::text::body(
                "La clave permite buscar y descargar metadatos y portadas desde ComicVine.",
            ))
            .push(api_key)
            .push(api_url)
            .push(
                widget::row::with_children(vec![test_api.into(), save_api.into()])
                    .spacing(8)
                    .align_y(Alignment::Center),
            )
            .push(widget::text::title2("Carpetas a escanear"))
            .push(widget::text::body(
                "Babelcomics buscará cómics dentro de todas estas carpetas.",
            ))
            .push(
                widget::row::with_children(vec![directory_input.into(), add_button.into()])
                    .spacing(8)
                    .align_y(Alignment::Center),
            )
            .push(choose_button)
            .spacing(8)
            .padding(12);
        if let Some(status) = &self.api_status {
            content = content.push(
                widget::container(widget::text::body(status))
                    .class(cosmic::theme::Container::Card)
                    .padding(10)
                    .width(Length::Fill),
            );
        }
        if let Some(error) = &self.directory_error {
            content = content.push(
                widget::container(widget::text::body(error))
                    .class(cosmic::theme::Container::Card)
                    .padding(10)
                    .width(Length::Fill),
            );
        }
        if self.stats.directories.is_empty() {
            content = content.push(widget::text::caption("No hay carpetas configuradas."));
        } else {
            for directory in &self.stats.directories {
                content = content.push(
                    widget::container(
                        widget::row::with_children(vec![
                            icon::from_name("folder-symbolic").size(20).icon().into(),
                            widget::text::body(&directory.path)
                                .width(Length::Fill)
                                .into(),
                            widget::tooltip(
                                widget::button::icon(icon::from_name("user-trash-symbolic"))
                                    .on_press_maybe(
                                        (!self.directory_saving)
                                            .then_some(Message::RemoveDirectory(directory.id)),
                                    )
                                    .class(cosmic::theme::Button::Destructive),
                                widget::text("Quitar carpeta"),
                                widget::tooltip::Position::Left,
                            )
                            .into(),
                        ])
                        .spacing(10)
                        .align_y(Alignment::Center),
                    )
                    .class(cosmic::theme::Container::Card)
                    .padding(10)
                    .width(Length::Fill),
                );
            }
        }
        let normalized_catalog_directory = self
            .stats
            .normalized_catalog_directory
            .as_deref()
            .unwrap_or("Sin configurar");
        content = content
            .push(widget::text::title2("Catálogo normalizado"))
            .push(widget::text::body(
                "Los cómics catalogados se moverán como Editorial/Volumen-Año/Volumen NNN.ext.",
            ))
            .push(self.info_row("Carpeta de destino", normalized_catalog_directory))
            .push(
                widget::button::standard("Elegir carpeta del catálogo")
                    .leading_icon(icon::from_name("folder-open-symbolic"))
                    .on_press_maybe(
                        (!self.preference_saving
                            && !self.organization_preparing
                            && !self.organizing)
                            .then_some(Message::ChooseNormalizedCatalogDirectory),
                    )
                    .width(Length::Fill),
            )
            .push(
                widget::button::suggested(if self.organization_preparing {
                    "Analizando biblioteca…"
                } else if self.organizing {
                    "Organizando catálogo…"
                } else {
                    "Preparar organización"
                })
                .leading_icon(icon::from_name("folder-new-symbolic"))
                .on_press_maybe(
                    (self.stats.normalized_catalog_directory.is_some()
                        && !self.organization_preparing
                        && !self.organizing)
                        .then_some(Message::PrepareOrganization),
                )
                .width(Length::Fill),
            );
        if !self.organization_plan.is_empty() {
            let movable = self
                .organization_plan
                .iter()
                .filter(|plan| {
                    matches!(
                        plan.status,
                        OrganizationStatus::Ok | OrganizationStatus::Duplicate
                    )
                })
                .count();
            let already_in_place = self
                .organization_plan
                .iter()
                .filter(|plan| plan.status == OrganizationStatus::AlreadyInPlace)
                .count();
            let uncatalogued = self
                .organization_plan
                .iter()
                .filter(|plan| plan.status == OrganizationStatus::NoInfo)
                .count();
            let plan_errors = self
                .organization_plan
                .iter()
                .filter(|plan| plan.status == OrganizationStatus::Error)
                .count();
            let summary = widget::column::with_children(vec![
                widget::text::title2("Plan listo para confirmar").into(),
                self.detail_value_row("Se moverán", movable.to_string()),
                self.detail_value_row("Ya organizados", already_in_place.to_string()),
                self.detail_value_row("Sin catalogar", uncatalogued.to_string()),
                self.detail_value_row("Con errores", plan_errors.to_string()),
                widget::text::caption(
                    "Los archivos sin catalogar no se mueven. Los nombres duplicados reciben _verXX.",
                )
                .into(),
                widget::row::with_children(vec![
                    widget::button::standard("Cancelar")
                        .on_press_maybe((!self.organizing).then_some(Message::CancelOrganization))
                        .width(Length::FillPortion(1))
                        .into(),
                    widget::button::suggested(if self.organizing {
                        "Moviendo…"
                    } else {
                        "Mover y organizar"
                    })
                    .on_press_maybe(
                        (movable > 0 && !self.organizing)
                            .then_some(Message::ConfirmOrganization),
                    )
                    .width(Length::FillPortion(2))
                    .into(),
                ])
                .spacing(8)
                .into(),
            ])
            .spacing(8);
            content = content.push(
                widget::container(summary)
                    .class(cosmic::theme::Container::Card)
                    .padding(12)
                    .width(Length::Fill),
            );
        }
        content = content
            .push(widget::text::title2("Tipos de archivo"))
            .push(widget::text::body(
                "Solo se incorporarán archivos con las extensiones marcadas.",
            ));
        let extension_checkbox = |label: &'static str, extension: &'static str| {
            widget::checkbox(self.stats.scan_extensions.contains(extension))
                .label(label)
                .on_toggle(move |enabled| Message::SetScanExtension(extension.to_string(), enabled))
                .width(Length::FillPortion(1))
        };
        content = content
            .push(
                widget::row::with_children(vec![
                    extension_checkbox("CBZ", "cbz").into(),
                    extension_checkbox("CBR", "cbr").into(),
                    extension_checkbox("CB7", "cb7").into(),
                    extension_checkbox("PDF", "pdf").into(),
                ])
                .spacing(8),
            )
            .push(
                widget::row::with_children(vec![
                    extension_checkbox("ZIP", "zip").into(),
                    extension_checkbox("RAR", "rar").into(),
                    extension_checkbox("7Z", "7z").into(),
                    widget::space().width(Length::FillPortion(1)).into(),
                ])
                .spacing(8),
            );
        content = content.push(scan_button);
        if let Some(status) = &self.scan_status {
            content = content.push(widget::text::caption(status));
        }
        content.into()
    }

    /// Guarda una preferencia del lector y recarga la configuración al terminar.
    fn save_setup_preference<F>(&mut self, save: F) -> Task<Message>
    where
        F: for<'r> FnOnce(
                SetupRepository<'r>,
            ) -> std::pin::Pin<
                Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + 'r>,
            > + Send
            + 'static,
    {
        if self.preference_saving {
            return Task::none();
        }
        self.preference_saving = true;
        self.preference_error = None;
        let pool = self.pool.clone();
        Task::perform(
            async move {
                save(SetupRepository::new(&pool))
                    .await
                    .map_err(|error| error.to_string())
            },
            |result| cosmic::Action::App(Message::PreferenceSaved(result)),
        )
    }

    fn reader_preferences_view(&self) -> Element<'_, Message> {
        let enabled = !self.preference_saving;
        let choice = |label: &'static str, active: bool, message: Message| {
            let button = if active {
                widget::button::suggested(label)
            } else {
                widget::button::standard(label)
            };
            button
                .on_press_maybe(enabled.then_some(message))
                .width(Length::FillPortion(1))
        };
        let choice_row = |buttons: Vec<Element<'static, Message>>| -> Element<'static, Message> {
            widget::row::with_children(buttons).spacing(8).into()
        };

        let stats = &self.stats;
        let executable = stats
            .reader_path
            .as_deref()
            .unwrap_or("Automático (junto a Babelcomics o en el PATH)");

        let fit = choice_row(vec![
            choice(
                "Página completa",
                stats.reader_fit == ReaderFit::Page,
                Message::SetReaderFit(ReaderFit::Page),
            )
            .into(),
            choice(
                "Ajustar al alto",
                stats.reader_fit == ReaderFit::Height,
                Message::SetReaderFit(ReaderFit::Height),
            )
            .into(),
            choice(
                "Ajustar al ancho",
                stats.reader_fit == ReaderFit::Width,
                Message::SetReaderFit(ReaderFit::Width),
            )
            .into(),
            choice(
                "Tamaño original",
                stats.reader_fit == ReaderFit::Original,
                Message::SetReaderFit(ReaderFit::Original),
            )
            .into(),
        ]);
        let direction = choice_row(vec![
            choice(
                "Izquierda → derecha",
                stats.reader_direction == ReaderDirection::Ltr,
                Message::SetReaderDirection(ReaderDirection::Ltr),
            )
            .into(),
            choice(
                "Derecha → izquierda (manga)",
                stats.reader_direction == ReaderDirection::Rtl,
                Message::SetReaderDirection(ReaderDirection::Rtl),
            )
            .into(),
        ]);
        let guided = choice_row(vec![
            choice(
                "Preguntar",
                stats.reader_guided == ReaderGuided::Ask,
                Message::SetReaderGuided(ReaderGuided::Ask),
            )
            .into(),
            choice(
                "Siempre",
                stats.reader_guided == ReaderGuided::Always,
                Message::SetReaderGuided(ReaderGuided::Always),
            )
            .into(),
            choice(
                "Nunca",
                stats.reader_guided == ReaderGuided::Never,
                Message::SetReaderGuided(ReaderGuided::Never),
            )
            .into(),
        ]);
        let filter = choice_row(
            [
                (0, "Nearest"),
                (1, "Triangle"),
                (2, "Catmull-Rom"),
                (3, "Lanczos3"),
            ]
            .into_iter()
            .map(|(value, label)| {
                // Valores fuera de rango se tratan como Lanczos3, igual que en core.
                let current = if (0..=2).contains(&stats.reader_filter) {
                    stats.reader_filter
                } else {
                    3
                };
                choice(label, current == value, Message::SetReaderFilter(value)).into()
            })
            .collect(),
        );

        let mut content = widget::column::with_children(vec![
            self.preferences_back_button(),
            widget::text::title2("Lector").into(),
            widget::text::body("Estas opciones se aplican la próxima vez que abras un cómic.").into(),
            widget::text::heading("Ejecutable").into(),
            self.info_row("Lector en uso", executable),
            widget::row::with_children(vec![
                widget::button::standard("Elegir ejecutable…")
                    .leading_icon(icon::from_name("document-open-symbolic"))
                    .on_press_maybe(enabled.then_some(Message::ChooseReaderExecutable))
                    .width(Length::FillPortion(1))
                    .into(),
                widget::button::standard("Usar automático")
                    .leading_icon(icon::from_name("edit-undo-symbolic"))
                    .on_press_maybe(
                        (enabled && stats.reader_path.is_some())
                            .then_some(Message::ResetReaderExecutable),
                    )
                    .width(Length::FillPortion(1))
                    .into(),
            ])
            .spacing(8)
            .into(),
            widget::text::heading("Ajuste de página").into(),
            fit,
            widget::text::heading("Sentido de lectura").into(),
            widget::text::caption("Define qué flecha del teclado avanza de página.").into(),
            direction,
            widget::text::heading("Lectura guiada").into(),
            widget::text::caption("Qué hacer cuando el cómic tiene viñetas definidas.").into(),
            guided,
            widget::text::heading("Calidad de escalado").into(),
            widget::text::caption("Filtro de las miniaturas de páginas. Nearest también muestra la página sin suavizado.").into(),
            filter,
        ])
        .spacing(8);
        if let Some(error) = &self.preference_error {
            content = content.push(widget::text::body(error.clone()));
        }
        content.into()
    }

    fn clip_toggle_row(
        &self,
        title: &'static str,
        description: &'static str,
        enabled: bool,
        on_toggle: fn(bool) -> Message,
    ) -> Element<'_, Message> {
        let toggler = widget::toggler(enabled);
        let toggler = if self.preference_saving {
            toggler
        } else {
            toggler.on_toggle(on_toggle)
        };
        widget::row::with_children(vec![
            widget::column::with_children(vec![
                widget::text::body(title).into(),
                widget::text::caption(description).into(),
            ])
            .spacing(2)
            .width(Length::Fill)
            .into(),
            toggler.into(),
        ])
        .spacing(12)
        .align_y(Alignment::Center)
        .into()
    }

    fn advanced_preferences_view(&self) -> Element<'_, Message> {
        let selected_size = match CardSize::from_db(self.stats.thumbnail_size) {
            CardSize::Small => 0,
            CardSize::Medium => 1,
            CardSize::Large => 2,
        };
        let size_button = |label, size| {
            let button = if selected_size == size {
                widget::button::suggested(label)
            } else {
                widget::button::standard(label)
            };
            button
                .on_press_maybe(
                    (!self.preference_saving).then_some(Message::SetThumbnailSize(size)),
                )
                .width(Length::FillPortion(1))
        };
        let sizes = widget::row::with_children(vec![
            size_button("Pequeña", 0).into(),
            size_button("Mediana", 1).into(),
            size_button("Grande", 2).into(),
        ])
        .spacing(8);

        let thumbnail_directory = self
            .stats
            .thumbnails_directory
            .as_deref()
            .unwrap_or("Predeterminada (~/.local/share/babelcomics/thumbnails)");
        let clip_status = if self.stats.clip_total == 0 {
            "No hay portadas de referencia".to_string()
        } else {
            format!(
                "{} indexadas · {} pendientes · {} portadas locales",
                self.stats.clip_indexed, self.stats.clip_pending, self.stats.clip_local
            )
        };
        let clip_button = widget::button::suggested(if self.clip_generating {
            "Generando embeddings…"
        } else {
            "Generar embeddings faltantes"
        })
        .leading_icon(icon::from_name("media-playback-start-symbolic"))
        .on_press_maybe((!self.clip_generating).then_some(Message::GenerateClipEmbeddings(None)))
        .width(Length::Fill);
        let choose_thumbnail_directory = widget::button::standard("Cambiar carpeta de miniaturas")
            .leading_icon(icon::from_name("folder-open-symbolic"))
            .on_press_maybe((!self.preference_saving).then_some(Message::ChooseThumbnailsDirectory))
            .width(Length::Fill);

        let mut content = widget::column::with_capacity(16)
            .push(self.preferences_back_button())
            .push(widget::text::title2("Miniaturas"))
            .push(widget::text::body("Tamaño generado para las portadas"))
            .push(sizes)
            .push(self.info_row("Carpeta de miniaturas", thumbnail_directory))
            .push(choose_thumbnail_directory)
            .push(widget::text::caption(
                "Las miniaturas actuales se moverán a la nueva ubicación; el destino debe estar vacío.",
            ))
            .push(widget::text::title2("Rendimiento"))
            .push(self.stat_row("Ítems por lote", self.stats.items_per_page))
            .push(self.stat_row("Workers configurados", self.stats.workers))
            .push(widget::text::title2("Índice visual CLIP"))
            .push(widget::text::body(
                "Calcula el índice de las portadas de referencia para la catalogación inteligente. La primera ejecución puede descargar un modelo de aproximadamente 340 MB.",
            ))
            .push(self.detail_value_row("Estado", clip_status))
            .push(clip_button)
            .push(self.clip_toggle_row(
                "Calcular al iniciar Babelcomics",
                "Genera los embeddings que falten cada vez que abres la aplicación.",
                self.stats.clip_on_startup,
                Message::SetClipOnStartup,
            ))
            .push(self.clip_toggle_row(
                "Calcular al bajar un volumen",
                "Genera los embeddings de las portadas nuevas al terminar de bajar o actualizar un volumen.",
                self.stats.clip_on_download,
                Message::SetClipOnDownload,
            ))
            .push(widget::text::caption(
                "Si los desactivas, puedes generarlos cuando quieras con el botón de arriba o desde el detalle de cada volumen.",
            ))
            .push(widget::text::title2("Diagnóstico de biblioteca"))
            .push(self.stat_row("Cómics", self.stats.comics))
            .push(self.stat_row("Sin catalogar", self.stats.uncatalogued))
            .push(self.stat_row("Con errores", self.stats.errors))
            .push(self.stat_row("Series", self.stats.volumes))
            .push(self.stat_row("Editoriales", self.stats.publishers))
            .push(self.info_row("Base de datos", &self.database_path))
            .spacing(8);

        if let Some(error) = &self.preference_error {
            content = content.push(
                widget::container(widget::text::body(error))
                    .class(cosmic::theme::Container::Card)
                    .padding(10)
                    .width(Length::Fill),
            );
        }
        content.into()
    }

    fn stat_row<'a>(&self, label: &'a str, value: i64) -> Element<'a, Message> {
        widget::container(widget::row::with_children(vec![
            widget::text::body(label).into(),
            widget::space().width(Length::Fill).into(),
            widget::text::body(value.to_string()).into(),
        ]))
        .class(cosmic::theme::Container::Card)
        .padding(10)
        .width(Length::Fill)
        .into()
    }

    fn info_row<'a>(&self, label: &'a str, value: &'a str) -> Element<'a, Message> {
        widget::container(
            widget::column::with_children(vec![
                widget::text::caption(label).into(),
                widget::text::body(value).into(),
            ])
            .spacing(3),
        )
        .class(cosmic::theme::Container::Card)
        .padding(10)
        .width(Length::Fill)
        .into()
    }

    fn comic_detail<'a>(&'a self, item: &'a ComicbookView) -> Element<'a, Message> {
        let filename = Path::new(&item.path)
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or(&item.path);
        let file_size = std::fs::metadata(&item.path)
            .map(|metadata| Self::format_bytes(metadata.len()))
            .unwrap_or_else(|_| "No disponible".to_string());
        let catalog_label = if item.titulo.is_some() {
            "Cambiar catalogación"
        } else {
            "Catalogar cómic"
        };
        let catalog = widget::button::suggested(catalog_label)
            .leading_icon(icon::from_name("system-search-symbolic"))
            .on_press(Message::OpenCatalog(Box::new(item.clone())))
            .width(Length::Fill);
        let read = widget::button::suggested("Leer cómic")
            .leading_icon(icon::from_name("media-playback-start-symbolic"))
            .on_press(Message::ReadComic(item.id_comicbook, item.path.clone()))
            .width(Length::Fill);
        let mut values = vec![
            (
                "Título".into(),
                item.titulo.as_deref().unwrap_or(filename).into(),
            ),
            (
                "Número".into(),
                item.numero.as_deref().unwrap_or("—").into(),
            ),
            (
                "Serie".into(),
                item.nombre_volume.as_deref().unwrap_or("Sin serie").into(),
            ),
            (
                "Editorial".into(),
                item.nombre_publisher
                    .as_deref()
                    .unwrap_or("Sin editorial")
                    .into(),
            ),
            (
                "Calidad".into(),
                item.calidad.as_deref().unwrap_or("—").into(),
            ),
            ("Tamaño".into(), file_size),
            ("Archivo".into(), item.path.clone()),
        ];
        if let Some(similarity) = item.catalog_match_similarity {
            let method = item
                .catalog_match_method
                .as_deref()
                .unwrap_or("desconocido")
                .to_uppercase();
            values.insert(
                4,
                (
                    "Catalogación".into(),
                    format!("{method} · {:.1}% de similitud", similarity * 100.0),
                ),
            );
        }
        let details = self.detail_list(
            values,
            DeleteTarget::Comic {
                id: item.id_comicbook,
                name: item.titulo.clone().unwrap_or_else(|| filename.to_string()),
                path: item.path.clone(),
            },
        );
        let mut content = widget::column::with_children(vec![read.into(), catalog.into(), details])
            .spacing(10)
            .padding(12);
        if let Some(error) = &self.reader_error {
            content = content.push(
                widget::container(widget::text::body(error))
                    .class(cosmic::theme::Container::Card)
                    .padding(10)
                    .width(Length::Fill),
            );
        }
        content.into()
    }

    fn comics_selection_detail<'a>(&'a self, items: &'a [ComicbookView]) -> Element<'a, Message> {
        let mut total_size = 0_u64;
        let mut types = BTreeMap::<&'static str, usize>::new();
        let mut series = BTreeSet::<&str>::new();
        let mut publishers = BTreeSet::<&str>::new();
        let mut directories = BTreeSet::new();
        let mut catalogued = 0_usize;

        for item in items {
            if let Ok(metadata) = std::fs::metadata(&item.path) {
                total_size = total_size.saturating_add(metadata.len());
            }
            let extension = Path::new(&item.path)
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            let mime = match extension.as_str() {
                "cbr" | "rar" => "application/vnd.comicbook-rar",
                "cbz" | "zip" => "application/vnd.comicbook+zip",
                "cb7" | "7z" => "application/x-7z-compressed",
                "pdf" => "application/pdf",
                _ => "application/octet-stream",
            };
            *types.entry(mime).or_default() += 1;
            if item.titulo.is_some() {
                catalogued += 1;
            }
            if let Some(name) = item.nombre_volume.as_deref() {
                series.insert(name);
            }
            if let Some(name) = item.nombre_publisher.as_deref() {
                publishers.insert(name);
            }
            if let Some(parent) = Path::new(&item.path).parent() {
                directories.insert(parent.to_string_lossy().into_owned());
            }
        }

        let type_summary = types
            .into_iter()
            .map(|(mime, count)| format!("{mime} ({count})"))
            .collect::<Vec<_>>()
            .join(" · ");
        let catalog_summary = format!(
            "{} catalogados · {} sin catalogar",
            catalogued,
            items.len().saturating_sub(catalogued)
        );
        let series_summary = Self::set_summary(&series, "Sin series");
        let publishers_summary = Self::set_summary(&publishers, "Sin editoriales");
        let directory_summary = if directories.len() == 1 {
            directories.into_iter().next().unwrap_or_default()
        } else {
            format!("{} ubicaciones", directories.len())
        };
        let delete_items = items
            .iter()
            .map(|item| (item.id_comicbook, item.path.clone()))
            .collect();

        let catalog = widget::button::suggested(format!("Catalogar {} cómics", items.len()))
            .leading_icon(icon::from_name("system-search-symbolic"))
            .on_press(Message::OpenCatalogBatch(items.to_vec()))
            .width(Length::Fill);
        let details = self.detail_list(
            vec![
                ("Archivos".into(), items.len().to_string()),
                ("Tipo".into(), type_summary),
                ("Tamaño total".into(), Self::format_bytes(total_size)),
                ("Catalogación".into(), catalog_summary),
                ("Series".into(), series_summary),
                ("Editoriales".into(), publishers_summary),
                ("Ubicación".into(), directory_summary),
            ],
            DeleteTarget::Comics {
                items: delete_items,
                label: format!("{} cómics", items.len()),
            },
        );
        widget::column::with_children(vec![catalog.into(), details])
            .spacing(10)
            .padding(12)
            .into()
    }

    fn volumes_selection_detail<'a>(&'a self, items: &'a [VolumeView]) -> Element<'a, Message> {
        let total_issues = items
            .iter()
            .map(|volume| volume.cantidad_numeros)
            .sum::<i64>();
        let owned_issues = items
            .iter()
            .map(|volume| volume.cantidad_poseida)
            .sum::<i64>();
        let updating = items
            .iter()
            .filter(|volume| {
                self.volume_updates
                    .values()
                    .any(|id| *id == volume.id_volume)
            })
            .count();

        let update_label = if updating == 0 {
            format!("Actualizar {} volúmenes", items.len())
        } else {
            format!(
                "Enviar {} volúmenes a la cola · {} ya en curso",
                items.len(),
                updating
            )
        };
        let update = widget::button::suggested(update_label)
            .leading_icon(icon::from_name("view-refresh-symbolic"))
            .on_press(Message::UpdateSelectedVolumes)
            .width(Length::Fill);

        let summary = widget::container(
            widget::column::with_children(vec![
                self.detail_value_row("Volúmenes seleccionados", items.len().to_string()),
                self.detail_value_row("Issues informados", total_issues.to_string()),
                self.detail_value_row(
                    "Issues catalogados",
                    format!("{owned_issues} de {total_issues}"),
                ),
                widget::text::caption(
                    "Las actualizaciones se ejecutan de a una y conservan el resto en cola. Incluyen metadatos, issues y portadas.",
                )
                .into(),
            ])
            .spacing(8),
        )
        .class(cosmic::theme::Container::Card)
        .padding(12)
        .width(Length::Fill);

        let mut volumes = widget::column::with_capacity(items.len()).spacing(8);
        for volume in items {
            volumes = volumes.push(self.detail_value_row(
                &volume.nombre,
                format!(
                    "{} · {} issues · {} catalogados",
                    volume.anio_inicio, volume.cantidad_numeros, volume.cantidad_poseida
                ),
            ));
        }

        widget::column::with_children(vec![
            update.into(),
            summary.into(),
            widget::text::heading("Detalle por volumen").into(),
            widget::scrollable(volumes).height(Length::Fill).into(),
        ])
        .spacing(10)
        .padding(12)
        .into()
    }

    fn set_summary(values: &BTreeSet<&str>, empty: &str) -> String {
        match values.len() {
            0 => empty.to_string(),
            1..=3 => values.iter().copied().collect::<Vec<_>>().join(", "),
            count => format!("{count} diferentes"),
        }
    }

    fn format_bytes(bytes: u64) -> String {
        const KIB: f64 = 1024.0;
        const MIB: f64 = KIB * 1024.0;
        const GIB: f64 = MIB * 1024.0;
        let bytes = bytes as f64;
        if bytes >= GIB {
            format!("{:.1} GB", bytes / GIB)
        } else if bytes >= MIB {
            format!("{:.1} MB", bytes / MIB)
        } else if bytes >= KIB {
            format!("{:.1} KB", bytes / KIB)
        } else {
            format!("{bytes:.0} B")
        }
    }

    fn catalog_view(&self) -> Element<'_, Message> {
        let Some(comic) = &self.catalog_comic else {
            return self.empty_view(
                "dialog-error-symbolic",
                "No hay un cómic seleccionado",
                "Vuelve a la biblioteca y selecciona un archivo.",
            );
        };
        let filename = Path::new(&comic.path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(&comic.path);
        let queue_status = if self.catalog_queue.len() > 1 {
            format!(
                "Archivo {} de {} · {filename}",
                self.catalog_queue_index + 1,
                self.catalog_queue.len()
            )
        } else {
            filename.to_string()
        };
        let back = widget::button::standard("Volver al cómic")
            .leading_icon(icon::from_name("go-previous-symbolic"))
            .on_press_maybe((!self.catalog_applying).then_some(Message::CloseCatalog));
        let header = widget::row::with_children(vec![
            back.into(),
            widget::column::with_children(vec![
                widget::text::title1("Catalogar cómic").into(),
                widget::text::caption(queue_status).into(),
            ])
            .spacing(2)
            .width(Length::Fill)
            .into(),
        ])
        .spacing(12)
        .align_y(Alignment::Center);

        let body: Element<_> = if self.catalog_manual {
            self.manual_catalog_view(comic)
        } else if self.catalog_loading {
            self.loading_view("Calculando similitudes de portada…")
        } else if let Some(error) = &self.catalog_error {
            widget::container(
                widget::column::with_children(vec![
                    icon::from_name("dialog-error-symbolic")
                        .size(48)
                        .icon()
                        .into(),
                    widget::text::title2("No se pudo catalogar").into(),
                    widget::text::body(error).into(),
                    widget::button::standard("Intentar nuevamente")
                        .leading_icon(icon::from_name("view-refresh-symbolic"))
                        .on_press(Message::Refresh)
                        .into(),
                    widget::button::suggested("Elegir serie manualmente")
                        .leading_icon(icon::from_name("edit-find-symbolic"))
                        .on_press(Message::OpenManualCatalog)
                        .into(),
                    widget::button::standard(self.catalog_skip_label())
                        .on_press(Message::SkipCatalog)
                        .into(),
                ])
                .spacing(10)
                .align_x(Alignment::Center),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center)
            .into()
        } else if self.catalog_candidates.is_empty() {
            widget::container(
                widget::column::with_children(vec![
                    icon::from_name("edit-find-symbolic").size(48).icon().into(),
                    widget::text::title2("Sin candidatos").into(),
                    widget::text::body("No se encontraron portadas similares. Genera primero el índice CLIP desde Preferencias avanzadas.").into(),
                    widget::button::suggested("Elegir serie manualmente")
                        .leading_icon(icon::from_name("edit-find-symbolic"))
                        .on_press(Message::OpenManualCatalog)
                        .into(),
                    widget::button::standard(self.catalog_skip_label())
                        .on_press(Message::SkipCatalog)
                        .into(),
                ])
                .spacing(10)
                .align_x(Alignment::Center),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center)
            .into()
        } else {
            self.catalog_candidates_view(comic)
        };

        widget::column::with_children(vec![header.into(), body])
            .spacing(12)
            .padding([8, 16])
            .height(Length::Fill)
            .into()
    }

    fn manual_catalog_view<'a>(&'a self, comic: &'a ComicbookView) -> Element<'a, Message> {
        let back_label = if self.manual_catalog_volume.is_some() {
            "Volver a series"
        } else {
            "Volver a sugerencias"
        };
        let back = widget::button::standard(back_label)
            .leading_icon(icon::from_name("go-previous-symbolic"))
            .on_press(Message::CloseManualCatalog);
        let heading = widget::column::with_children(vec![
            widget::text::title2("Catalogación manual").into(),
            widget::text::caption(
                Path::new(&comic.path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(&comic.path),
            )
            .into(),
        ])
        .spacing(2);
        let header = widget::row::with_children(vec![back.into(), heading.into()])
            .spacing(10)
            .align_y(Alignment::Center);

        let body: Element<_> = if self.manual_catalog_volume.is_some() {
            if self.manual_catalog_loading {
                self.loading_view("Cargando números y portadas…")
            } else if let Some(error) = &self.manual_catalog_error {
                self.empty_view(
                    "dialog-error-symbolic",
                    "No se pudieron cargar los números",
                    error,
                )
            } else if self.catalog_candidates.is_empty() {
                self.empty_view(
                    "x-office-document-symbolic",
                    "Serie sin números",
                    "Actualiza esta serie desde ComicVine antes de catalogar el archivo.",
                )
            } else {
                self.catalog_candidates_view(comic)
            }
        } else {
            self.manual_catalog_volumes_view()
        };
        widget::column::with_children(vec![header.into(), body])
            .spacing(12)
            .height(Length::Fill)
            .into()
    }

    fn manual_catalog_volumes_view(&self) -> Element<'_, Message> {
        let search = widget::search_input("Buscar serie…", &self.manual_catalog_query)
            .on_input(Message::ManualCatalogQueryChanged)
            .on_clear(Message::ManualCatalogQueryChanged(String::new()))
            .width(Length::Fill);
        let results: Element<_> = if self.manual_catalog_loading {
            self.loading_view("Buscando series…")
        } else if let Some(error) = &self.manual_catalog_error {
            self.empty_view(
                "dialog-error-symbolic",
                "No se pudieron cargar las series",
                error,
            )
        } else if self.manual_catalog_volumes.is_empty() {
            self.empty_view(
                "edit-find-symbolic",
                "Sin series",
                "Prueba con otro nombre.",
            )
        } else {
            widget::responsive(|size| {
                let columns = ((size.width / 230.0).floor() as usize).clamp(2, 6);
                let mut rows = widget::column::with_capacity(
                    self.manual_catalog_volumes.len().div_ceil(columns),
                )
                .spacing(12);
                for chunk in self.manual_catalog_volumes.chunks(columns) {
                    let mut row = widget::row::with_capacity(columns).spacing(12);
                    for volume in chunk {
                        row = row.push(self.manual_catalog_volume_card(volume));
                    }
                    for _ in chunk.len()..columns {
                        row = row.push(widget::space().width(Length::Fill));
                    }
                    rows = rows.push(row);
                }
                widget::scrollable(rows.padding([4, 2]))
                    .height(Length::Fill)
                    .into()
            })
            .into()
        };
        widget::column::with_children(vec![
            widget::text::body("Busca y selecciona la serie correcta.").into(),
            search.into(),
            widget::text::caption(format!(
                "{} series encontradas",
                self.manual_catalog_volumes.len()
            ))
            .into(),
            results,
        ])
        .spacing(8)
        .height(Length::Fill)
        .into()
    }

    fn manual_catalog_volume_card<'a>(&self, volume: &'a VolumeView) -> Element<'a, Message> {
        let cover = volume_thumbnail_path(volume.id_volume);
        let visual: Element<_> = if cover.exists() {
            widget::image(ImageHandle::from_path(cover))
                .width(Length::Fill)
                .height(150)
                .content_fit(cosmic::iced::ContentFit::Contain)
                .into()
        } else {
            widget::container(icon::from_name("image-missing-symbolic").size(36).icon())
                .width(Length::Fill)
                .height(150)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center)
                .class(cosmic::theme::Container::Secondary)
                .into()
        };
        widget::button::custom(
            widget::container(
                widget::column::with_children(vec![
                    visual,
                    widget::text::body(&volume.nombre).into(),
                    widget::text::caption(format!(
                        "{} · {} números",
                        volume.anio_inicio, volume.cantidad_numeros
                    ))
                    .into(),
                ])
                .spacing(6),
            )
            .class(cosmic::theme::Container::Card)
            .padding(8)
            .width(Length::Fill),
        )
        .on_press(Message::SelectManualCatalogVolume(volume.id_volume))
        .class(cosmic::theme::Button::Transparent)
        .padding(0)
        .width(Length::FillPortion(1))
        .into()
    }

    fn catalog_candidates_view<'a>(&'a self, comic: &'a ComicbookView) -> Element<'a, Message> {
        let local_cover = comic_thumbnail_path(comic.id_comicbook, CardSize::Large);
        let target_visual: Element<_> = if local_cover.exists() {
            widget::image(cosmic::iced::widget::image::Handle::from_path(local_cover))
                .width(150)
                .height(210)
                .content_fit(cosmic::iced::ContentFit::Contain)
                .into()
        } else {
            widget::container(icon::from_name("image-missing-symbolic").size(48).icon())
                .width(150)
                .height(210)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center)
                .class(cosmic::theme::Container::Secondary)
                .into()
        };
        let target = widget::container(
            widget::row::with_children(vec![
                target_visual,
                widget::column::with_children(vec![
                    widget::text::title2("Archivo a catalogar").into(),
                    widget::text::body(
                        Path::new(&comic.path)
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or(&comic.path),
                    )
                    .into(),
                    widget::text::caption("Elige abajo la portada que corresponde a este archivo.")
                        .into(),
                ])
                .spacing(6)
                .width(Length::Fill)
                .into(),
            ])
            .spacing(18)
            .align_y(Alignment::Center),
        )
        .class(cosmic::theme::Container::Card)
        .padding(14)
        .width(Length::Fill);

        let candidates_title = if self.catalog_manual && self.manual_catalog_volume.is_some() {
            "Números de la serie seleccionada"
        } else {
            "Coincidencias sugeridas"
        };
        let mut candidates = widget::column::with_capacity(self.catalog_candidates.len() + 2)
            .push(widget::text::title2(candidates_title))
            .spacing(10);
        for (index, candidate) in self.catalog_candidates.iter().enumerate() {
            candidates = candidates.push(self.catalog_candidate_card(candidate, index));
        }
        let selected = &self.catalog_candidates[self.catalog_selected];
        let selected_number = selected.numero.as_deref().unwrap_or("?");
        let confirm_label = if self.catalog_applying {
            "Guardando catalogación…"
        } else if self.catalog_manual && self.manual_catalog_volume.is_some() {
            "Catalogar con este número"
        } else {
            "Confirmar catalogación"
        };
        let confirm = widget::button::suggested(confirm_label)
            .leading_icon(icon::from_name("emblem-ok-symbolic"))
            .on_press_maybe((!self.catalog_applying).then_some(Message::ApplyCatalog))
            .width(Length::Fill);
        let actions = widget::row::with_children(vec![
            widget::button::standard(self.catalog_skip_label())
                .on_press_maybe((!self.catalog_applying).then_some(Message::SkipCatalog))
                .width(Length::FillPortion(1))
                .into(),
            confirm.width(Length::FillPortion(2)).into(),
        ])
        .spacing(8);
        let footer = widget::container(
            widget::column::with_children(vec![
                widget::text::body(format!(
                    "Seleccionado: {} #{} — {}",
                    selected
                        .nombre_volume
                        .as_deref()
                        .unwrap_or("Serie desconocida"),
                    selected_number,
                    selected.titulo
                ))
                .into(),
                actions.into(),
            ])
            .spacing(8),
        )
        .class(cosmic::theme::Container::Card)
        .padding(12)
        .width(Length::Fill);

        widget::column::with_children(vec![
            target.into(),
            widget::button::standard(if self.catalog_manual {
                "Elegir otra serie"
            } else {
                "Elegir otra serie manualmente"
            })
            .leading_icon(icon::from_name("edit-find-symbolic"))
            .on_press(if self.catalog_manual {
                Message::CloseManualCatalog
            } else {
                Message::OpenManualCatalog
            })
            .width(Length::Fill)
            .into(),
            widget::scrollable(candidates.padding([4, 2]))
                .height(Length::Fill)
                .into(),
            footer.into(),
        ])
        .spacing(12)
        .height(Length::Fill)
        .into()
    }

    fn catalog_skip_label(&self) -> &'static str {
        if self.catalog_queue_index + 1 < self.catalog_queue.len() {
            "Omitir y continuar"
        } else {
            "Omitir y finalizar"
        }
    }

    fn catalog_candidate_card<'a>(
        &self,
        candidate: &'a UnifiedSuggestion,
        index: usize,
    ) -> Element<'a, Message> {
        let selected = self.catalog_selected == index;
        let cover = self
            .catalog_candidate_covers
            .get(&candidate.id_comicbook_info)
            .cloned();
        let visual: Element<_> = if let Some(handle) = cover {
            widget::image(handle)
                .width(90)
                .height(126)
                .content_fit(cosmic::iced::ContentFit::Contain)
                .into()
        } else {
            widget::container(icon::from_name("image-missing-symbolic").size(36).icon())
                .width(90)
                .height(126)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center)
                .class(cosmic::theme::Container::Secondary)
                .into()
        };
        let match_caption = if self.catalog_manual && self.manual_catalog_volume.is_some() {
            format!("Selección manual · candidato {}", index + 1)
        } else {
            let method = match candidate.method {
                SuggestionMethod::Clip => "CLIP",
                SuggestionMethod::Hash => "Hash",
            };
            format!(
                "{method} · {:.1}% de similitud · candidato {}",
                candidate.similarity.clamp(0.0, 1.0) * 100.0,
                index + 1
            )
        };
        let number = candidate.numero.as_deref().unwrap_or("?");
        let content = widget::row::with_children(vec![
            visual,
            widget::column::with_children(vec![
                widget::text::title2(
                    candidate
                        .nombre_volume
                        .as_deref()
                        .unwrap_or("Serie desconocida"),
                )
                .into(),
                widget::text::body(format!("#{number} — {}", candidate.titulo)).into(),
                widget::text::caption(match_caption).into(),
            ])
            .spacing(5)
            .width(Length::Fill)
            .into(),
            if selected {
                icon::from_name("emblem-ok-symbolic").size(24).icon().into()
            } else {
                widget::space().width(24).into()
            },
        ])
        .spacing(14)
        .align_y(Alignment::Center);
        widget::button::custom(
            widget::container(content)
                .class(if selected {
                    cosmic::theme::Container::Primary
                } else {
                    cosmic::theme::Container::Card
                })
                .padding(10)
                .width(Length::Fill),
        )
        .on_press_maybe((!self.catalog_applying).then_some(Message::SelectCatalogCandidate(index)))
        .selected(selected)
        .class(cosmic::theme::Button::Transparent)
        .padding(0)
        .width(Length::Fill)
        .into()
    }

    fn issue_detail_view(&self) -> Element<'_, Message> {
        let back = widget::button::standard("Volver a números")
            .leading_icon(icon::from_name("go-previous-symbolic"))
            .on_press(Message::CloseIssueDetail);
        if self.issue_detail_loading {
            return widget::column::with_children(vec![
                back.into(),
                self.loading_view("Cargando detalle de los números…"),
            ])
            .spacing(12)
            .padding([8, 16])
            .height(Length::Fill)
            .into();
        }
        let Some(detail) = &self.issue_detail else {
            return widget::column::with_children(vec![
                back.into(),
                self.empty_view(
                    "dialog-error-symbolic",
                    "No se pudo abrir el detalle",
                    self.issue_detail_error
                        .as_deref()
                        .unwrap_or("No hay números seleccionados."),
                ),
            ])
            .spacing(12)
            .padding([8, 16])
            .height(Length::Fill)
            .into();
        };
        let title = if detail.issues.len() == 1 {
            let issue = &detail.issues[0];
            format!(
                "#{} — {}",
                issue.info.numero.as_deref().unwrap_or("?"),
                issue.info.titulo
            )
        } else {
            format!("{} números seleccionados", detail.issues.len())
        };
        let information = if self.issue_detail_tab == IssueDetailTab::Information {
            widget::button::suggested("Información")
        } else {
            widget::button::standard("Información")
        }
        .leading_icon(icon::from_name("dialog-information-symbolic"))
        .on_press(Message::SelectIssueDetailTab(IssueDetailTab::Information))
        .width(Length::FillPortion(1));
        let cover_count = detail.issue_covers.values().map(Vec::len).sum::<usize>();
        let covers = if self.issue_detail_tab == IssueDetailTab::Covers {
            widget::button::suggested(format!("Portadas ({cover_count})"))
        } else {
            widget::button::standard(format!("Portadas ({cover_count})"))
        }
        .leading_icon(icon::from_name("image-x-generic-symbolic"))
        .on_press(Message::SelectIssueDetailTab(IssueDetailTab::Covers))
        .width(Length::FillPortion(1));
        let physical = if self.issue_detail_tab == IssueDetailTab::PhysicalComics {
            widget::button::suggested(format!("Cómics físicos ({})", detail.physical_comics.len()))
        } else {
            widget::button::standard(format!("Cómics físicos ({})", detail.physical_comics.len()))
        }
        .leading_icon(icon::from_name("x-office-document-symbolic"))
        .on_press(Message::SelectIssueDetailTab(
            IssueDetailTab::PhysicalComics,
        ))
        .width(Length::FillPortion(1));
        let body = match self.issue_detail_tab {
            IssueDetailTab::Information => self.issue_information_view(detail),
            IssueDetailTab::Covers => self.issue_covers_view(detail),
            IssueDetailTab::PhysicalComics => self.issue_physical_comics_view(detail),
        };
        widget::column::with_children(vec![
            widget::row::with_children(vec![
                back.into(),
                widget::text::title1(title).width(Length::Fill).into(),
            ])
            .spacing(12)
            .align_y(Alignment::Center)
            .into(),
            widget::row::with_children(vec![information.into(), covers.into(), physical.into()])
                .spacing(8)
                .into(),
            body,
        ])
        .spacing(12)
        .padding([8, 16])
        .height(Length::Fill)
        .into()
    }

    fn issue_information_view<'a>(&'a self, detail: &'a IssueDetailData) -> Element<'a, Message> {
        let owned = detail
            .issues
            .iter()
            .filter(|issue| issue.physical_count > 0)
            .count();
        let mut content = widget::column::with_children(vec![
            self.detail_value_row("Números seleccionados", detail.issues.len().to_string()),
            self.detail_value_row("Con archivos físicos", owned.to_string()),
            self.detail_value_row(
                "Archivos relacionados",
                detail.physical_comics.len().to_string(),
            ),
        ])
        .spacing(10);
        for issue in &detail.issues {
            let mut values = widget::column::with_children(vec![
                widget::text::heading(format!(
                    "#{} — {}",
                    issue.info.numero.as_deref().unwrap_or("?"),
                    issue.info.titulo
                ))
                .into(),
                self.detail_value_row("ID del issue", issue.info.id_comicbook_info.to_string()),
                self.detail_value_row("Cómics físicos", issue.physical_count.to_string()),
                self.detail_value_row(
                    "Calificación",
                    issue
                        .info
                        .calificacion
                        .map(|rating| format!("{rating:.1}"))
                        .unwrap_or_else(|| "—".to_string()),
                ),
                self.detail_value_row(
                    "ComicVine",
                    issue
                        .info
                        .id_comicvine
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| "Sin vincular".to_string()),
                ),
            ])
            .spacing(6);
            if let Some(summary) = issue
                .info
                .resumen
                .as_deref()
                .filter(|text| !text.is_empty())
            {
                values = values
                    .push(widget::text::heading("Resumen"))
                    .push(widget::text::body(summary));
            }
            if let Some(url) = comicvine_issue_page_url(issue) {
                values = values.push(
                    widget::button::standard("Abrir issue en ComicVine")
                        .leading_icon(icon::from_name("web-browser-symbolic"))
                        .on_press(Message::OpenExternalUrl(url))
                        .width(Length::Fill),
                );
            }
            content = content.push(
                widget::container(values)
                    .class(cosmic::theme::Container::Card)
                    .padding(12)
                    .width(Length::Fill),
            );
        }
        widget::scrollable(content.padding([4, 2]))
            .height(Length::Fill)
            .into()
    }

    fn issue_covers_view<'a>(&'a self, detail: &'a IssueDetailData) -> Element<'a, Message> {
        let covers = detail
            .issues
            .iter()
            .flat_map(|issue| {
                detail
                    .issue_covers
                    .get(&issue.info.id_comicbook_info)
                    .into_iter()
                    .flatten()
                    .enumerate()
                    .map(move |(index, cover)| (issue, index, cover))
            })
            .collect::<Vec<_>>();
        if covers.is_empty() {
            return self.empty_view(
                "image-missing-symbolic",
                "Sin portadas",
                "No hay portadas descargadas para los números seleccionados.",
            );
        }

        widget::responsive(move |size| {
            let columns = ((size.width / 230.0).floor() as usize).clamp(1, 6);
            let mut rows =
                widget::column::with_capacity(covers.len().div_ceil(columns)).spacing(14);
            for chunk in covers.chunks(columns) {
                let mut row = widget::row::with_capacity(columns).spacing(14);
                for (issue, index, cover) in chunk {
                    let number = issue.info.numero.as_deref().unwrap_or("?");
                    let label = if detail
                        .issue_covers
                        .get(&issue.info.id_comicbook_info)
                        .is_some_and(|covers| covers.len() > 1)
                    {
                        format!("#{number} · Portada {}", index + 1)
                    } else {
                        format!("#{number} · Portada")
                    };
                    let card = widget::container(
                        widget::column::with_children(vec![
                            widget::image((*cover).clone())
                                .width(Length::Fill)
                                .height(300)
                                .content_fit(cosmic::iced::ContentFit::Contain)
                                .into(),
                            widget::text::body(label).into(),
                            widget::text::caption(&issue.info.titulo).into(),
                        ])
                        .spacing(6),
                    )
                    .class(cosmic::theme::Container::Card)
                    .padding(8)
                    .width(Length::Fill);
                    row = row.push(card);
                }
                for _ in chunk.len()..columns {
                    row = row.push(widget::space().width(Length::Fill));
                }
                rows = rows.push(row);
            }
            widget::scrollable(rows.padding([4, 2]))
                .height(Length::Fill)
                .into()
        })
        .into()
    }

    fn issue_physical_comics_view<'a>(
        &'a self,
        detail: &'a IssueDetailData,
    ) -> Element<'a, Message> {
        if detail.physical_comics.is_empty() {
            return self.empty_view(
                "x-office-document-symbolic",
                "Sin cómics físicos",
                "Los números seleccionados no tienen archivos clasificados.",
            );
        }
        let mut content = widget::column::with_capacity(detail.physical_comics.len()).spacing(8);
        if let Some(error) = &self.issue_detail_error {
            content = content.push(
                widget::container(widget::text::body(error))
                    .class(cosmic::theme::Container::Card)
                    .padding(10)
                    .width(Length::Fill),
            );
        }
        if let Some(error) = &self.reader_error {
            content = content.push(
                widget::container(widget::text::body(error))
                    .class(cosmic::theme::Container::Card)
                    .padding(10)
                    .width(Length::Fill),
            );
        }
        for (issue_id, comic) in &detail.physical_comics {
            let issue = detail
                .issues
                .iter()
                .find(|issue| issue.info.id_comicbook_info == *issue_id);
            let filename = Path::new(&comic.path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(&comic.path);
            let issue_label = issue.map_or_else(
                || "Issue desconocido".to_string(),
                |issue| {
                    format!(
                        "#{} — {}",
                        issue.info.numero.as_deref().unwrap_or("?"),
                        issue.info.titulo
                    )
                },
            );
            let thumbnail = comic_thumbnail_path(comic.id_comicbook, CardSize::Medium);
            let total_pages = comic.reading_total_pages.max(0);
            let visited_pages = comic.reading_visited_pages.clamp(0, total_pages);
            let completion = if total_pages > 0 {
                (visited_pages as f32 / total_pages as f32).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let has_progress = visited_pages > 0;
            let completed = total_pages > 0 && visited_pages == total_pages;
            let reading_status = if completed {
                "LEÍDO"
            } else if has_progress {
                "LEYENDO"
            } else {
                "SIN EMPEZAR"
            };
            let progress_description = if total_pages > 0 {
                format!(
                    "{visited_pages} de {total_pages} páginas · {:.0}%",
                    completion * 100.0
                )
            } else {
                "Todavía no hay progreso de lectura".to_string()
            };
            let read_label = if completed {
                "Releer"
            } else if has_progress {
                "Continuar"
            } else {
                "Leer ahora"
            };
            let visual: Element<_> = if thumbnail.exists() {
                widget::image(cosmic::iced::widget::image::Handle::from_path(thumbnail))
                    .width(110)
                    .height(154)
                    .content_fit(cosmic::iced::ContentFit::Contain)
                    .into()
            } else {
                widget::container(icon::from_name("image-missing-symbolic").size(36).icon())
                    .width(110)
                    .height(154)
                    .align_x(Horizontal::Center)
                    .align_y(Vertical::Center)
                    .class(cosmic::theme::Container::Secondary)
                    .into()
            };
            content = content.push(
                widget::container(
                    widget::row::with_children(vec![
                        visual,
                        widget::column::with_children(vec![
                            widget::text::heading(filename).into(),
                            widget::text::body(issue_label).into(),
                            widget::text::caption(&comic.path).into(),
                            self.detail_value_row(
                                "Calidad",
                                comic.calidad.as_deref().unwrap_or("—").to_string(),
                            ),
                            widget::column::with_children(vec![
                                widget::row::with_children(vec![
                                    widget::text::caption_heading(reading_status).into(),
                                    widget::space().width(Length::Fill).into(),
                                    widget::text::caption(format!("{:.0}%", completion * 100.0))
                                        .into(),
                                ])
                                .align_y(Alignment::Center)
                                .into(),
                                widget::progress_bar::linear::Linear::new()
                                    .progress(completion)
                                    .girth(4)
                                    .width(Length::Fill)
                                    .into(),
                                widget::text::caption(progress_description).into(),
                            ])
                            .spacing(5)
                            .into(),
                            widget::row::with_children(vec![
                                widget::button::suggested(read_label)
                                    .leading_icon(icon::from_name("media-playback-start-symbolic"))
                                    .on_press(Message::ReadComic(
                                        comic.id_comicbook,
                                        comic.path.clone(),
                                    ))
                                    .width(Length::FillPortion(1))
                                    .into(),
                                widget::button::destructive("Desclasificar")
                                    .leading_icon(icon::from_name("edit-clear-symbolic"))
                                    .on_press(Message::UnlinkPhysicalComic(comic.id_comicbook))
                                    .width(Length::FillPortion(1))
                                    .into(),
                            ])
                            .spacing(8)
                            .into(),
                        ])
                        .spacing(6)
                        .width(Length::Fill)
                        .into(),
                    ])
                    .spacing(12)
                    .align_y(Alignment::Start),
                )
                .class(cosmic::theme::Container::Card)
                .padding(12)
                .width(Length::Fill),
            );
        }
        widget::scrollable(content.padding([4, 2]))
            .height(Length::Fill)
            .into()
    }

    fn volume_detail_view(&self) -> Element<'_, Message> {
        let back = widget::button::standard("Volver a series")
            .leading_icon(icon::from_name("go-previous-symbolic"))
            .on_press(Message::CloseVolumeDetail);

        if self.volume_detail_loading {
            return widget::column::with_children(vec![
                back.into(),
                self.loading_view("Cargando detalle de la serie…"),
            ])
            .spacing(12)
            .padding([8, 16])
            .height(Length::Fill)
            .into();
        }

        let Some(detail) = &self.volume_detail else {
            let error = self
                .volume_detail_error
                .as_deref()
                .unwrap_or("No se pudo cargar la serie");
            return widget::column::with_children(vec![
                back.into(),
                self.empty_view(
                    "dialog-error-symbolic",
                    "No se pudo abrir el detalle",
                    error,
                ),
            ])
            .spacing(12)
            .padding([8, 16])
            .height(Length::Fill)
            .into();
        };

        let information = if self.volume_detail_tab == VolumeDetailTab::Information {
            widget::button::suggested("Información")
        } else {
            widget::button::standard("Información")
        }
        .leading_icon(icon::from_name("dialog-information-symbolic"))
        .on_press(Message::SelectVolumeDetailTab(VolumeDetailTab::Information))
        .width(Length::FillPortion(1));
        let issues = if self.volume_detail_tab == VolumeDetailTab::Issues {
            widget::button::suggested(format!("Números ({})", detail.issues.len()))
        } else {
            widget::button::standard(format!("Números ({})", detail.issues.len()))
        }
        .leading_icon(icon::from_name("view-grid-symbolic"))
        .on_press(Message::SelectVolumeDetailTab(VolumeDetailTab::Issues))
        .width(Length::FillPortion(1));

        let title = widget::row::with_children(vec![
            back.into(),
            widget::text::title1(&detail.volume.nombre)
                .width(Length::Fill)
                .into(),
        ])
        .spacing(12)
        .align_y(Alignment::Center);
        let tabs = widget::row::with_children(vec![information.into(), issues.into()])
            .spacing(8)
            .width(Length::Fill);
        let content = match self.volume_detail_tab {
            VolumeDetailTab::Information => self.volume_information_view(detail),
            VolumeDetailTab::Issues => self.volume_issues_view(detail),
        };

        widget::column::with_children(vec![title.into(), tabs.into(), content])
            .spacing(12)
            .padding([8, 16])
            .height(Length::Fill)
            .into()
    }

    fn volume_information_view<'a>(&'a self, detail: &'a VolumeDetailData) -> Element<'a, Message> {
        let volume = &detail.volume;
        let owned = detail
            .issues
            .iter()
            .filter(|issue| issue.physical_count > 0)
            .count() as i64;
        let total = volume.cantidad_numeros.max(0);
        let completion = if total > 0 {
            (owned as f32 / total as f32).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let cover_handle = self.volume_detail_cover(detail);
        let cover: Element<_> = if let Some(handle) = cover_handle {
            widget::image(handle)
                .width(220)
                .height(310)
                .content_fit(cosmic::iced::ContentFit::Contain)
                .into()
        } else {
            widget::container(icon::from_name("image-missing-symbolic").size(64).icon())
                .width(220)
                .height(310)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center)
                .class(cosmic::theme::Container::Secondary)
                .into()
        };

        let publisher = detail
            .publisher
            .as_ref()
            .map(|publisher| publisher.nombre.as_str())
            .unwrap_or("Sin editorial");
        let mut identifiers = vec![
            ("ID del volumen", volume.id_volume.to_string()),
            ("Año de inicio", volume.anio_inicio.to_string()),
            ("Editorial", publisher.to_string()),
        ];
        if let Some(id) = volume.id_comicvine {
            identifiers.insert(1, ("ID ComicVine", id.to_string()));
        }
        let mut stats = widget::column::with_capacity(identifiers.len() + 5)
            .push(widget::text::title2("Estadísticas de colección"))
            .push(widget::text::body(format!(
                "{owned} de {total} cómics ({:.1}%)",
                completion * 100.0
            )))
            .push(
                widget::progress_bar::linear::Linear::new()
                    .progress(completion)
                    .girth(10.0)
                    .width(Length::Fill),
            )
            .spacing(8);
        for (label, value) in identifiers {
            stats = stats.push(self.detail_value_row(label, value));
        }
        if let Some(url) = comicvine_volume_page_url(volume) {
            stats = stats.push(
                widget::button::standard("Abrir volumen en ComicVine")
                    .leading_icon(icon::from_name("web-browser-symbolic"))
                    .on_press(Message::OpenExternalUrl(url))
                    .width(Length::Fill),
            );
        }
        let stats = widget::container(stats)
            .class(cosmic::theme::Container::Card)
            .padding(16)
            .width(Length::Fill);
        let header = widget::row::with_children(vec![cover, stats.into()])
            .spacing(20)
            .align_y(Alignment::Start);

        let mut content = widget::column::with_capacity(6).push(header).spacing(18);
        if !volume.deck.trim().is_empty() {
            content = content.push(self.volume_text_section("Resumen", &volume.deck));
        }
        if !volume.descripcion.trim().is_empty() && volume.descripcion != volume.deck {
            content = content.push(self.volume_text_section("Descripción", &volume.descripcion));
        }

        let clip_status = if detail.clip_total == 0 {
            "No hay portadas candidatas en este volumen".to_string()
        } else if detail.clip_pending == 0 {
            format!(
                "{} portadas indexadas de {} disponibles localmente",
                detail.clip_indexed, detail.clip_local
            )
        } else {
            format!(
                "{} indexadas · {} pendientes · {} portadas locales",
                detail.clip_indexed, detail.clip_pending, detail.clip_local
            )
        };
        let comicvine_status = if volume.id_comicvine.is_some() {
            "Vinculado; se pueden actualizar metadatos, números y portadas".to_string()
        } else {
            "Sin ID de ComicVine asignado".to_string()
        };
        let is_updating = volume
            .id_comicvine
            .is_some_and(|cv_id| self.volume_updates.contains_key(&cv_id));
        let update_button = widget::button::suggested(if is_updating {
            "Actualizando volumen…"
        } else {
            "Actualizar volumen"
        })
        .leading_icon(icon::from_name("view-refresh-symbolic"))
        .on_press_maybe(
            (volume.id_comicvine.is_some() && !is_updating).then_some(Message::UpdateVolume),
        )
        .width(Length::Fill);
        let clip_button = widget::button::suggested(if self.clip_generating {
            "Generando embeddings…"
        } else if detail.clip_pending == 0 {
            "Índice CLIP actualizado"
        } else {
            "Generar embeddings faltantes"
        })
        .leading_icon(icon::from_name(if detail.clip_pending == 0 {
            "emblem-ok-symbolic"
        } else {
            "media-playback-start-symbolic"
        }))
        .on_press_maybe(
            (!self.clip_generating && detail.clip_pending > 0)
                .then_some(Message::GenerateClipEmbeddings(Some(volume.id_volume))),
        )
        .width(Length::Fill);
        let tools = widget::column::with_children(vec![
            widget::text::title2("Herramientas").into(),
            self.detail_value_row("ComicVine", comicvine_status),
            update_button.into(),
            self.detail_value_row("Índice visual CLIP", clip_status),
            clip_button.into(),
        ])
        .spacing(8);
        content = content.push(
            widget::container(tools)
                .class(cosmic::theme::Container::Card)
                .padding(16)
                .width(Length::Fill),
        );
        content = content.push(
            widget::button::destructive("Eliminar volumen")
                .leading_icon(icon::from_name("user-trash-symbolic"))
                .on_press(Message::RequestDelete(Box::new(DeleteTarget::Volume {
                    id: volume.id_volume,
                    name: volume.nombre.clone(),
                }))),
        );

        widget::scrollable(content.padding([4, 2]))
            .height(Length::Fill)
            .into()
    }

    fn volume_issues_view<'a>(&'a self, detail: &'a VolumeDetailData) -> Element<'a, Message> {
        let search =
            widget::search_input("Filtrar por número o título…", &self.volume_issue_search)
                .on_input(Message::VolumeIssueSearchChanged)
                .on_clear(Message::VolumeIssueSearchChanged(String::new()))
                .width(Length::Fill);
        let all = self.issue_filter_button("Todos", IssueOwnershipFilter::All);
        let owned = self.issue_filter_button("Poseídos", IssueOwnershipFilter::Owned);
        let missing = self.issue_filter_button("No poseídos", IssueOwnershipFilter::Missing);
        let filters = widget::row::with_children(vec![all, owned, missing]).spacing(6);
        let detail_button = widget::button::suggested(if self.selected_issues.is_empty() {
            "Selecciona números"
        } else {
            "Ver detalle (Ctrl+Espacio)"
        })
        .leading_icon(icon::from_name("dialog-information-symbolic"))
        .on_press_maybe(
            (!self.selected_issues.is_empty()).then_some(Message::OpenSelectedIssueDetail),
        );
        let toolbar = widget::column::with_children(vec![
            widget::row::with_children(vec![search.into(), filters.into()])
                .spacing(10)
                .align_y(Alignment::Center)
                .into(),
            detail_button.into(),
        ])
        .spacing(8);

        let query = self.volume_issue_search.trim().to_lowercase();
        let filtered = detail
            .issues
            .iter()
            .filter(|issue| {
                let ownership_matches = match self.volume_issue_filter {
                    IssueOwnershipFilter::All => true,
                    IssueOwnershipFilter::Owned => issue.physical_count > 0,
                    IssueOwnershipFilter::Missing => issue.physical_count == 0,
                };
                let query_matches = query.is_empty()
                    || issue.info.titulo.to_lowercase().contains(&query)
                    || issue
                        .info
                        .numero
                        .as_deref()
                        .is_some_and(|number| number.to_lowercase().contains(&query));
                ownership_matches && query_matches
            })
            .collect::<Vec<_>>();
        let count = widget::text::caption(if self.selected_issues.is_empty() {
            format!("{} números · Ctrl+clic selecciona varios", filtered.len())
        } else {
            format!(
                "{} números · {} seleccionados",
                filtered.len(),
                self.selected_issues.len()
            )
        });
        let grid: Element<_> = if filtered.is_empty() {
            self.empty_view(
                "edit-find-symbolic",
                "Sin resultados",
                "No hay números que coincidan con este filtro.",
            )
        } else {
            widget::responsive(move |size| {
                let columns = ((size.width / 190.0).floor() as usize).clamp(2, 8);
                let mut rows =
                    widget::column::with_capacity(filtered.len().div_ceil(columns)).spacing(14);
                for chunk in filtered.chunks(columns) {
                    let mut row = widget::row::with_capacity(columns).spacing(14);
                    for issue in chunk {
                        let covers = detail.issue_covers.get(&issue.info.id_comicbook_info);
                        let cover = covers.and_then(|covers| covers.first());
                        row = row.push(self.volume_issue_card(
                            issue,
                            cover,
                            covers.map_or(0, Vec::len),
                            self.selected_issues.contains(&issue.info.id_comicbook_info),
                        ));
                    }
                    for _ in chunk.len()..columns {
                        row = row.push(widget::space().width(Length::Fill));
                    }
                    rows = rows.push(row);
                }
                widget::scrollable(rows.padding([4, 2]))
                    .height(Length::Fill)
                    .into()
            })
            .into()
        };

        widget::column::with_children(vec![toolbar.into(), count.into(), grid])
            .spacing(10)
            .height(Length::Fill)
            .into()
    }

    fn issue_filter_button(
        &self,
        label: &'static str,
        filter: IssueOwnershipFilter,
    ) -> Element<'_, Message> {
        let button = if self.volume_issue_filter == filter {
            widget::button::suggested(label)
        } else {
            widget::button::standard(label)
        };
        button
            .on_press(Message::SetIssueOwnershipFilter(filter))
            .into()
    }

    fn volume_issue_card<'a>(
        &self,
        issue: &'a ComicbookInfoView,
        cover: Option<&ImageHandle>,
        cover_count: usize,
        selected: bool,
    ) -> Element<'a, Message> {
        let visual: Element<_> = if let Some(handle) = cover.cloned() {
            widget::image(handle)
                .width(Length::Fill)
                .height(240)
                .content_fit(cosmic::iced::ContentFit::Contain)
                .into()
        } else {
            widget::container(icon::from_name("image-missing-symbolic").size(44).icon())
                .width(Length::Fill)
                .height(240)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center)
                .class(cosmic::theme::Container::Secondary)
                .into()
        };
        let number = issue.info.numero.as_deref().unwrap_or("?");
        let mut card = widget::column::with_children(vec![
            visual,
            widget::text::body(&issue.info.titulo).into(),
            widget::text::caption(format!("#{number}")).into(),
        ])
        .spacing(6);
        if cover_count > 1 {
            card = card.push(widget::text::caption(format!("{cover_count} portadas")));
        }
        if selected {
            card = card.push(widget::text::caption("Seleccionado"));
        }
        widget::button::custom(
            widget::container(card)
                .class(if selected {
                    cosmic::theme::Container::Primary
                } else if issue.physical_count > 0 {
                    cosmic::theme::Container::Card
                } else {
                    cosmic::theme::Container::Secondary
                })
                .padding(8)
                .width(Length::Fill),
        )
        .on_press(Message::IssueClicked(issue.info.id_comicbook_info))
        .selected(selected)
        .class(cosmic::theme::Button::Transparent)
        .padding(0)
        .width(Length::FillPortion(1))
        .into()
    }

    fn volume_detail_cover(&self, detail: &VolumeDetailData) -> Option<ImageHandle> {
        let volume_cover = volume_thumbnail_path(detail.volume.id_volume);
        if volume_cover.exists() {
            return Some(ImageHandle::from_path(volume_cover));
        }
        detail.issues.iter().find_map(|issue| {
            detail
                .issue_covers
                .get(&issue.info.id_comicbook_info)
                .and_then(|covers| covers.first())
                .cloned()
        })
    }

    fn volume_text_section<'a>(&self, title: &'a str, value: &'a str) -> Element<'a, Message> {
        widget::container(
            widget::column::with_children(vec![
                widget::text::title2(title).into(),
                widget::text::body(value).into(),
            ])
            .spacing(8),
        )
        .class(cosmic::theme::Container::Card)
        .padding(16)
        .width(Length::Fill)
        .into()
    }

    fn detail_value_row<'a>(&self, label: &'a str, value: String) -> Element<'a, Message> {
        widget::container(
            widget::column::with_children(vec![
                widget::text::caption(label).into(),
                widget::text::body(value).into(),
            ])
            .spacing(2),
        )
        .class(cosmic::theme::Container::Secondary)
        .padding(10)
        .width(Length::Fill)
        .into()
    }

    fn publisher_detail<'a>(&'a self, item: &'a Publisher) -> Element<'a, Message> {
        self.detail_list(
            vec![
                ("Editorial".into(), item.nombre.clone()),
                (
                    "Descripción".into(),
                    item.descripcion
                        .as_deref()
                        .unwrap_or("Sin descripción")
                        .into(),
                ),
            ],
            DeleteTarget::Publisher {
                id: item.id_publisher,
                name: item.nombre.clone(),
            },
        )
    }

    fn detail_list(
        &self,
        values: Vec<(String, String)>,
        delete_target: DeleteTarget,
    ) -> Element<'_, Message> {
        let mut content = widget::column::with_capacity(values.len() + 2)
            .spacing(8)
            .padding(12);
        for (label, value) in values {
            content = content.push(
                widget::container(
                    widget::column::with_children(vec![
                        widget::text::caption(label).into(),
                        widget::text::body(value).into(),
                    ])
                    .spacing(3),
                )
                .class(cosmic::theme::Container::Card)
                .padding(10)
                .width(Length::Fill),
            );
        }
        content = content.push(widget::space::vertical().height(8)).push(
            widget::button::destructive(format!("Eliminar {}", delete_target.kind()))
                .leading_icon(icon::from_name("user-trash-symbolic"))
                .on_press(Message::RequestDelete(Box::new(delete_target)))
                .width(Length::Fill),
        );
        content.into()
    }
}
