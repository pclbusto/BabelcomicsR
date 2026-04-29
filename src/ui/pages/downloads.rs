use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{self as gtk, glib};
use libadwaita as adw;
use sqlx::SqlitePool;

use babelcomics_core::helpers::download_manager::{DownloadManager, DownloadStatus};

struct DownloadRow {
    row: adw::ActionRow,
    progress: gtk::ProgressBar,
    status_icon: gtk::Image,
}

pub fn build(pool: SqlitePool) -> gtk::Widget {
    let toolbar = adw::ToolbarView::new();

    let scrolled = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hexpand(true)
        .build();

    let list_box = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(vec!["boxed-list".to_string()])
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    let placeholder = adw::StatusPage::builder()
        .title("Sin descargas activas")
        .description("Las descargas de metadatos y portadas de ComicVine aparecerán aquí")
        .icon_name("folder-download-symbolic")
        .build();

    let stack = gtk::Stack::new();
    stack.add_named(&placeholder, Some("empty"));
    stack.add_named(&scrolled, Some("list"));
    stack.set_visible_child_name("empty");

    scrolled.set_child(Some(&list_box));
    toolbar.set_content(Some(&stack));

    // Mapa de filas por cv_id
    let rows: Rc<RefCell<HashMap<String, DownloadRow>>> = Rc::new(RefCell::new(HashMap::new()));

    let dm = DownloadManager::get_instance(pool);
    let state = dm.get_state();

    // Polling cada 250 ms
    let list_box_weak = list_box.downgrade();
    let stack_weak = stack.downgrade();
    let rows_clone = rows.clone();

    glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
        let (list_box, stack) = match (list_box_weak.upgrade(), stack_weak.upgrade()) {
            (Some(l), Some(s)) => (l, s),
            _ => return glib::ControlFlow::Break,
        };

        let downloads = state.lock().unwrap();
        let mut rows_map = rows_clone.borrow_mut();

        // Añadir filas nuevas
        for (cv_id, info) in downloads.iter() {
            if !rows_map.contains_key(cv_id) {
                let row = adw::ActionRow::builder()
                    .title(glib::markup_escape_text(&info.title).as_str())
                    .subtitle(glib::markup_escape_text(&info.message).as_str())
                    .build();

                let status_icon = gtk::Image::builder()
                    .icon_name(status_icon_name(&info.status))
                    .pixel_size(20)
                    .build();
                row.add_prefix(&status_icon);

                let progress = gtk::ProgressBar::builder()
                    .fraction(info.progress)
                    .valign(gtk::Align::Center)
                    .width_request(120)
                    .build();
                row.add_suffix(&progress);

                list_box.append(&row);
                stack.set_visible_child_name("list");

                rows_map.insert(
                    cv_id.clone(),
                    DownloadRow {
                        row,
                        progress,
                        status_icon,
                    },
                );
            }
        }

        // Actualizar filas existentes
        for (cv_id, dr) in rows_map.iter() {
            if let Some(info) = downloads.get(cv_id) {
                dr.row
                    .set_subtitle(glib::markup_escape_text(&info.message).as_str());
                dr.progress.set_fraction(info.progress);
                dr.status_icon
                    .set_icon_name(Some(status_icon_name(&info.status)));

                if matches!(info.status, DownloadStatus::Error(_)) {
                    dr.row.add_css_class("error");
                } else {
                    dr.row.remove_css_class("error");
                }
            }
        }

        glib::ControlFlow::Continue
    });

    toolbar.upcast()
}

fn status_icon_name(status: &DownloadStatus) -> &'static str {
    match status {
        DownloadStatus::Queued => "emblem-system-symbolic",
        DownloadStatus::Downloading => "folder-download-symbolic",
        DownloadStatus::Completed => "emblem-ok-symbolic",
        DownloadStatus::Error(_) => "dialog-error-symbolic",
    }
}
