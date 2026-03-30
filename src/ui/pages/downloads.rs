use gtk4::prelude::*;
use gtk4::{self as gtk};
use libadwaita as adw;

pub fn build() -> gtk::Widget {
    let toolbar = adw::ToolbarView::new();

    let scrolled = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hexpand(true)
        .build();

    // Lista de descargas activas
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

    toolbar.upcast()
}
