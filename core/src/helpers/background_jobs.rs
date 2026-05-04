use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

#[derive(Clone, Debug, PartialEq)]
pub enum JobKind {
    Download,
    Clip,
    Scan,
    Thumbnail,
    Import,
}

#[derive(Clone, Debug)]
pub enum BackgroundJobStatus {
    Running,
    Completed,
    Error,
}

pub struct BackgroundJob {
    pub title: String,
    pub message: String,
    pub progress: f64,
    pub status: BackgroundJobStatus,
    pub kind: JobKind,
    pub created_at: Instant,
    pub finished_at: Option<Instant>,
}

/// Vista inmutable de un job, devuelta por `snapshot()`.
#[derive(Clone, Debug)]
pub struct BackgroundJobSnapshot {
    pub id: String,
    pub title: String,
    pub message: String,
    pub progress: f64,
    pub status: BackgroundJobStatus,
    pub kind: JobKind,
    pub created_at: Instant,
    pub finished_at: Option<Instant>,
}

/// Jobs completados se eliminan del registro tras 5 minutos.
/// Errores persisten hasta que la app se cierra.
const COMPLETED_TTL: Duration = Duration::from_secs(300);

static JOBS: OnceLock<Arc<Mutex<HashMap<String, BackgroundJob>>>> = OnceLock::new();
static NEXT_JOB_ID: AtomicU64 = AtomicU64::new(1);
static NOTIFY_TX: OnceLock<broadcast::Sender<()>> = OnceLock::new();

fn jobs() -> Arc<Mutex<HashMap<String, BackgroundJob>>> {
    JOBS.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
        .clone()
}

fn notify_tx() -> &'static broadcast::Sender<()> {
    NOTIFY_TX.get_or_init(|| {
        let (tx, _) = broadcast::channel(32);
        tx
    })
}

/// Suscribe un receptor a las notificaciones de cambio de jobs.
/// El receptor recibe `()` cada vez que un job es creado, actualizado o finalizado.
pub fn subscribe_jobs() -> broadcast::Receiver<()> {
    notify_tx().subscribe()
}

pub fn start_job(title: impl Into<String>, message: impl Into<String>, kind: JobKind) -> String {
    let id = format!("job-{}", NEXT_JOB_ID.fetch_add(1, Ordering::Relaxed));
    jobs().lock().unwrap().insert(
        id.clone(),
        BackgroundJob {
            title: title.into(),
            message: message.into(),
            progress: 0.0,
            status: BackgroundJobStatus::Running,
            kind,
            created_at: Instant::now(),
            finished_at: None,
        },
    );
    let _ = notify_tx().send(());
    id
}

pub fn update_job(id: &str, message: impl Into<String>, progress: f64) {
    let updated = {
        let state = jobs();
        let mut map = state.lock().unwrap();
        if let Some(job) = map.get_mut(id) {
            job.message = message.into();
            job.progress = progress.clamp(0.0, 1.0);
            job.status = BackgroundJobStatus::Running;
            true
        } else {
            false
        }
    };
    if updated {
        let _ = notify_tx().send(());
    }
}

pub fn finish_job(id: &str, message: impl Into<String>, has_error: bool) {
    let finished = {
        let state = jobs();
        let mut map = state.lock().unwrap();
        if let Some(job) = map.get_mut(id) {
            job.message = message.into();
            job.progress = if has_error { job.progress } else { 1.0 };
            job.status = if has_error {
                BackgroundJobStatus::Error
            } else {
                BackgroundJobStatus::Completed
            };
            job.finished_at = Some(Instant::now());
            true
        } else {
            false
        }
    };
    if finished {
        let _ = notify_tx().send(());
    }
}

/// Devuelve una vista ordenada cronológicamente de todos los jobs activos.
/// Limpia los completados que superaron el TTL; los errores persisten.
pub fn snapshot() -> Vec<BackgroundJobSnapshot> {
    let state = jobs();
    let mut map = state.lock().unwrap();
    let now = Instant::now();

    map.retain(|_, job| match job.status {
        BackgroundJobStatus::Completed => job
            .finished_at
            .map_or(true, |t| now.duration_since(t) < COMPLETED_TTL),
        _ => true,
    });

    let mut result: Vec<BackgroundJobSnapshot> = map
        .iter()
        .map(|(id, job)| BackgroundJobSnapshot {
            id: id.clone(),
            title: job.title.clone(),
            message: job.message.clone(),
            progress: job.progress,
            status: job.status.clone(),
            kind: job.kind.clone(),
            created_at: job.created_at,
            finished_at: job.finished_at,
        })
        .collect();

    result.sort_by_key(|j| j.created_at);
    result
}
