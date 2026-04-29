pub mod pages;
pub mod preferences_dialog;
pub mod reader;
pub mod window;

pub use preferences_dialog::build_preferences_dialog;

use std::cell::RefCell;
use std::future::Future;
use std::rc::Rc;

use gtk4::glib;

/// Ejecuta `task` en tokio y llama `on_done` en el hilo de GTK cuando termina.
/// `task` solo puede capturar tipos Send; `on_done` puede capturar widgets GTK.
pub fn run_in_background<T, F, D>(rt: tokio::runtime::Handle, task: F, on_done: D)
where
    T: Send + 'static,
    F: Future<Output = T> + Send + 'static,
    D: FnOnce(T) + 'static,
{
    let (tx, rx) = std::sync::mpsc::sync_channel::<T>(1);
    let rx = Rc::new(RefCell::new(rx));

    glib::timeout_add_local(std::time::Duration::from_millis(50), {
        let mut cb = Some(on_done);
        move || match rx.borrow().try_recv() {
            Ok(val) => {
                if let Some(f) = cb.take() {
                    f(val);
                }
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });

    rt.spawn(async move {
        let result = task.await;
        let _ = tx.send(result);
    });
}
