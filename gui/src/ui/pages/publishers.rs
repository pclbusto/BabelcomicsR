use gtk4::prelude::*;
use gtk4::{self as gtk, ScrolledWindow, SearchEntry};
use libadwaita as adw;

const EDITORIAL_ICON_PATH: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/editorial.svg");

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

    let placeholder = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .spacing(12)
        .build();
    placeholder.append(
        &gtk::Image::builder()
            .file(EDITORIAL_ICON_PATH)
            .pixel_size(96)
            .opacity(0.65)
            .build(),
    );
    placeholder.append(
        &gtk::Label::builder()
            .label("Sin editoriales")
            .css_classes(["title-1"])
            .build(),
    );
    placeholder.append(
        &gtk::Label::builder()
            .label("Las editoriales aparecerán al descargar metadatos de ComicVine")
            .css_classes(["dim-label"])
            .wrap(true)
            .justify(gtk::Justification::Center)
            .build(),
    );

    let stack = gtk::Stack::new();
    stack.add_named(&placeholder, Some("empty"));
    stack.add_named(&scrolled, Some("grid"));
    stack.set_visible_child_name("empty");

    scrolled.set_child(Some(&flow));
    toolbar.set_content(Some(&stack));

    toolbar.upcast()
}
