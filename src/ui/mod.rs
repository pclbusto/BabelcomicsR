pub mod pages;
pub mod preferences_dialog;
pub mod reader;
pub mod window;

pub use preferences_dialog::build_preferences_dialog;

use std::future::Future;

use gtk4::glib;

/// Ejecuta `task` en tokio y llama `on_done` en el hilo de GTK cuando termina.
/// `task` solo puede capturar tipos Send; `on_done` puede capturar widgets GTK.
pub fn run_in_background<T, F, D>(rt: tokio::runtime::Handle, task: F, on_done: D)
where
    T: Send + 'static,
    F: Future<Output = T> + Send + 'static,
    D: FnOnce(T) + 'static,
{
    let (tx, rx) = tokio::sync::oneshot::channel::<T>();

    rt.spawn(async move {
        let result = task.await;
        let _ = tx.send(result);
    });

    glib::MainContext::default().spawn_local(async move {
        if let Ok(val) = rx.await {
            on_done(val);
        }
    });
}
