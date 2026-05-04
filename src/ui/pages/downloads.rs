use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{self as gtk, glib};
use libadwaita as adw;
use sqlx::SqlitePool;
use tokio::sync::broadcast::error::RecvError;

use babelcomics_core::helpers::background_jobs::{
    BackgroundJobStatus, JobKind, snapshot, subscribe_jobs,
};

struct StatusRow {
    row: adw::ActionRow,
    progress: gtk::ProgressBar,
    status_icon: gtk::Image,
}

pub fn build(_pool: SqlitePool) -> gtk::Widget {
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
        .title("Sin actividad")
        .description("Las descargas, la indexación CLIP y otras tareas aparecerán aquí")
        .icon_name("folder-download-symbolic")
        .build();

    let stack = gtk::Stack::new();
    stack.add_named(&placeholder, Some("empty"));
    stack.add_named(&scrolled, Some("list"));
    stack.set_visible_child_name("empty");

    scrolled.set_child(Some(&list_box));
    toolbar.set_content(Some(&stack));

    let rows: Rc<RefCell<HashMap<String, StatusRow>>> = Rc::new(RefCell::new(HashMap::new()));

    let list_box_weak = list_box.downgrade();
    let stack_weak = stack.downgrade();
    let rows_clone = rows.clone();

    render_jobs_snapshot(&list_box, &stack, &rows);

    // Future local en el hilo GTK: se suspende esperando notificaciones del broadcast
    // y actualiza la UI cuando llega un evento real de cambio de job.
    glib::MainContext::default().spawn_local(async move {
        let mut rx = subscribe_jobs();
        loop {
            match rx.recv().await {
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            }

            let (list_box, stack) = match (list_box_weak.upgrade(), stack_weak.upgrade()) {
                (Some(l), Some(s)) => (l, s),
                _ => break,
            };

            render_jobs_snapshot(&list_box, &stack, &rows_clone);
        }
    });

    toolbar.upcast()
}

fn render_jobs_snapshot(
    list_box: &gtk::ListBox,
    stack: &gtk::Stack,
    rows: &Rc<RefCell<HashMap<String, StatusRow>>>,
) {
    let jobs = snapshot();
    let mut rows_map = rows.borrow_mut();

    for job in &jobs {
        if !rows_map.contains_key(&job.id) {
            let row = build_status_row(
                &job.title,
                &job.message,
                icon_name(&job.kind, &job.status),
                job.progress,
            );
            list_box.append(&row.row);
            rows_map.insert(job.id.clone(), row);
        }
        if let Some(sr) = rows_map.get(&job.id) {
            sr.row
                .set_subtitle(glib::markup_escape_text(&job.message).as_str());
            sr.progress.set_fraction(job.progress);
            sr.status_icon
                .set_icon_name(Some(icon_name(&job.kind, &job.status)));

            if matches!(job.status, BackgroundJobStatus::Error) {
                sr.row.add_css_class("error");
            } else {
                sr.row.remove_css_class("error");
            }
        }
    }

    // Eliminar filas de jobs que ya no están en el snapshot (purgados por TTL)
    let live_ids: std::collections::HashSet<&str> = jobs.iter().map(|j| j.id.as_str()).collect();
    rows_map.retain(|id, row| {
        if live_ids.contains(id.as_str()) {
            true
        } else {
            list_box.remove(&row.row);
            false
        }
    });

    if rows_map.is_empty() {
        stack.set_visible_child_name("empty");
    } else {
        stack.set_visible_child_name("list");
    }
}

fn build_status_row(title: &str, message: &str, icon: &str, fraction: f64) -> StatusRow {
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(title).as_str())
        .subtitle(glib::markup_escape_text(message).as_str())
        .build();

    let status_icon = gtk::Image::builder().icon_name(icon).pixel_size(20).build();
    row.add_prefix(&status_icon);

    let progress = gtk::ProgressBar::builder()
        .fraction(fraction)
        .valign(gtk::Align::Center)
        .width_request(120)
        .build();
    row.add_suffix(&progress);

    StatusRow {
        row,
        progress,
        status_icon,
    }
}

fn icon_name(kind: &JobKind, status: &BackgroundJobStatus) -> &'static str {
    match status {
        BackgroundJobStatus::Completed => "emblem-ok-symbolic",
        BackgroundJobStatus::Error => "dialog-error-symbolic",
        BackgroundJobStatus::Running => match kind {
            JobKind::Download => "folder-download-symbolic",
            JobKind::Clip => "brain-augemnted-symbolic",
            JobKind::Scan => "folder-open-symbolic",
            JobKind::Thumbnail => "image-x-generic-symbolic",
            JobKind::Import => "document-import-symbolic",
        },
    }
}
