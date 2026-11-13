pub mod pages;
pub mod preferences_dialog;
pub mod reader;
pub mod window;

pub use preferences_dialog::build_preferences_dialog;

use std::future::Future;
use std::panic::Location;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use std::{cell::Cell, cell::RefCell, rc::Rc};

use gtk4::glib;

static NEXT_BACKGROUND_TASK_ID: AtomicUsize = AtomicUsize::new(1);
static ACTIVE_BACKGROUND_TASKS: AtomicUsize = AtomicUsize::new(0);

struct PendingUiTask {
    task_id: usize,
    caller: String,
    completed: Rc<Cell<bool>>,
    handle: glib::JoinHandle<()>,
}

thread_local! {
    static PENDING_UI_TASKS: RefCell<Vec<PendingUiTask>> = const { RefCell::new(Vec::new()) };
}

pub fn active_background_tasks() -> usize {
    ACTIVE_BACKGROUND_TASKS.load(Ordering::Relaxed)
}

pub fn log_pending_background_callbacks(reason: &str) {
    PENDING_UI_TASKS.with(|tasks| {
        let tasks = tasks.borrow();
        let pending: Vec<_> = tasks
            .iter()
            .filter(|task| !task.completed.get())
            .map(|task| (task.task_id, task.caller.as_str()))
            .collect();
        tracing::debug!(
            reason,
            pending = pending.len(),
            total_handles = tasks.len(),
            active = ACTIVE_BACKGROUND_TASKS.load(Ordering::Relaxed),
            pending_tasks = ?pending,
            "Callbacks GTK pendientes"
        );
    });
}

pub fn abort_pending_background_callbacks(reason: &str) {
    PENDING_UI_TASKS.with(|tasks| {
        let mut tasks = tasks.borrow_mut();
        let pending = tasks.iter().filter(|task| !task.completed.get()).count();
        tracing::debug!(
            reason,
            pending,
            total_handles = tasks.len(),
            active = ACTIVE_BACKGROUND_TASKS.load(Ordering::Relaxed),
            "Abortando callbacks GTK pendientes"
        );

        for task in tasks.drain(..) {
            if !task.completed.get() {
                task.handle.abort();
                let remaining = ACTIVE_BACKGROUND_TASKS
                    .fetch_sub(1, Ordering::Relaxed)
                    .saturating_sub(1);
                tracing::debug!(
                    task_id = task.task_id,
                    caller = %task.caller,
                    active = remaining,
                    "run_in_background: callback abortada"
                );
            }
        }
    });
}

fn forget_completed_background_callback(task_id: usize) {
    PENDING_UI_TASKS.with(|tasks| {
        tasks.borrow_mut().retain(|task| task.task_id != task_id);
    });
}

pub fn install_main_loop_stall_monitor() {
    thread_local! {
        static INSTALLED: Cell<bool> = const { Cell::new(false) };
    }

    INSTALLED.with(|installed| {
        if installed.replace(true) {
            return;
        }

        let mut last_tick = Instant::now();
        glib::timeout_add_local(Duration::from_millis(250), move || {
            let now = Instant::now();
            let elapsed = now.duration_since(last_tick);
            if elapsed > Duration::from_millis(750) {
                tracing::warn!(
                    elapsed_ms = elapsed.as_millis(),
                    "GTK main loop bloqueado o saturado"
                );
            }
            last_tick = now;
            glib::ControlFlow::Continue
        });
    });
}

/// Ejecuta `task` en tokio y llama `on_done` en el hilo de GTK cuando termina.
/// `task` solo puede capturar tipos Send; `on_done` puede capturar widgets GTK.
#[track_caller]
pub fn run_in_background<T, F, D>(rt: tokio::runtime::Handle, task: F, on_done: D)
where
    T: Send + 'static,
    F: Future<Output = T> + Send + 'static,
    D: FnOnce(T) + 'static,
{
    let caller = Location::caller();
    let caller = format!("{}:{}", caller.file(), caller.line());
    let task_id = NEXT_BACKGROUND_TASK_ID.fetch_add(1, Ordering::Relaxed);
    ACTIVE_BACKGROUND_TASKS.fetch_add(1, Ordering::Relaxed);
    tracing::debug!(
        task_id,
        caller = %caller,
        active = ACTIVE_BACKGROUND_TASKS.load(Ordering::Relaxed),
        "run_in_background: tarea iniciada"
    );

    let (tx, rx) = tokio::sync::oneshot::channel::<T>();
    let caller_task = caller.clone();
    let caller_done = caller.clone();
    let completed = Rc::new(Cell::new(false));
    let completed_done = completed.clone();
    let started = Instant::now();

    rt.spawn(async move {
        let result = task.await;
        let elapsed = started.elapsed();
        tracing::debug!(
            task_id,
            caller = %caller_task,
            elapsed_ms = elapsed.as_millis(),
            "run_in_background: future terminado"
        );
        let _ = tx.send(result);
    });

    let handle = glib::MainContext::default().spawn_local(async move {
        if let Ok(val) = rx.await {
            let callback_started = Instant::now();
            on_done(val);
            let callback_elapsed = callback_started.elapsed();
            completed_done.set(true);
            let remaining = ACTIVE_BACKGROUND_TASKS
                .fetch_sub(1, Ordering::Relaxed)
                .saturating_sub(1);
            tracing::debug!(
                task_id,
                caller = %caller_done,
                active = remaining,
                callback_ms = callback_elapsed.as_millis(),
                "run_in_background: callback liberada"
            );
        } else {
            completed_done.set(true);
            let remaining = ACTIVE_BACKGROUND_TASKS
                .fetch_sub(1, Ordering::Relaxed)
                .saturating_sub(1);
            tracing::debug!(
                task_id,
                caller = %caller_done,
                active = remaining,
                "run_in_background: callback cancelada antes de recibir resultado"
            );
        }
        forget_completed_background_callback(task_id);
    });

    PENDING_UI_TASKS.with(|tasks| {
        tasks.borrow_mut().push(PendingUiTask {
            task_id,
            caller,
            completed,
            handle,
        });
    });
}
