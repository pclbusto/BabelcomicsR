use gtk4::prelude::*;
use gtk4::{self as gtk, ScrolledWindow, SearchEntry};
use libadwaita as adw;

pub fn build() -> gtk::Widget {
    let toolbar = adw::ToolbarView::new();

    let search_bar = gtk::SearchBar::new();
    let search_entry = SearchEntry::new();
    search_entry.set_hexpand(true);
    search_entry.set_placeholder_text(Some("Buscar editoriales…"));
    search_bar.set_child(Some(&search_entry));
    search_bar.set_show_close_button(true);
    toolbar.add_top_bar(&search_bar);

    let scrolled = ScrolledWindow::builder()
        .vexpand(true)
        .hexpand(true)
        .build();

    let flow = gtk::FlowBox::builder()
        .valign(gtk::Align::Start)
        .max_children_per_line(10)
        .min_children_per_line(2)
        .selection_mode(gtk::SelectionMode::Multiple)
        .row_spacing(12)
        .column_spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    let placeholder = adw::StatusPage::builder()
        .title("Sin editoriales")
        .description("Las editoriales aparecerán al descargar metadatos de ComicVine")
        .icon_name("building-symbolic")
        .build();

    let stack = gtk::Stack::new();
    stack.add_named(&placeholder, Some("empty"));
    stack.add_named(&scrolled, Some("grid"));
    stack.set_visible_child_name("empty");

    scrolled.set_child(Some(&flow));
    toolbar.set_content(Some(&stack));

    toolbar.upcast()
}
