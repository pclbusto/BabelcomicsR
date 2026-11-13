//! Panel lateral con la lista de atajos, filtrable por texto y categoría.

use cosmic::Element;
use cosmic::iced::{Alignment, Length};
use cosmic::widget::{self, column, container, row, settings, text, text_input};

use crate::Message;
use crate::shortcuts::{self, Category, Shortcut};

/// Etiquetas del filtro: "Todas" seguida de cada categoría.
pub const CATEGORY_FILTERS: [&str; 5] =
    ["Todas", "Navegación", "Vista", "Editor de viñetas", "Ayuda"];

fn filter_category(filter: usize) -> Option<Category> {
    filter
        .checked_sub(1)
        .and_then(|i| Category::ALL.get(i).copied())
}

pub fn view_header<'a>(query: &'a str, filter: usize) -> Element<'a, Message> {
    row![
        text_input::search_input("Buscar un atajo…", query)
            .on_input(Message::ShortcutsQueryChanged)
            .on_clear(Message::ShortcutsQueryChanged(String::new()))
            .width(Length::Fill),
        widget::dropdown(
            &CATEGORY_FILTERS[..],
            Some(filter),
            Message::ShortcutsFilterSelected
        ),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .into()
}

pub fn view_content<'a>(query: &str, filter: usize) -> Element<'a, Message> {
    let all = shortcuts::all();
    let wanted = filter_category(filter);
    let mut sections = column![].spacing(16);
    let mut any = false;

    for category in Category::ALL {
        if wanted.is_some_and(|c| c != category) {
            continue;
        }
        let items: Vec<&Shortcut> = all
            .iter()
            .filter(|s| s.category == category && s.matches_query(query))
            .collect();
        if items.is_empty() {
            continue;
        }
        any = true;
        let mut section = settings::section().title(category.label());
        for shortcut in items {
            section = section.add(shortcut_row(shortcut));
        }
        sections = sections.push(section);
    }

    if !any {
        sections = sections.push(
            container(text::body("No hay atajos que coincidan"))
                .width(Length::Fill)
                .center_x(Length::Fill)
                .padding(24),
        );
    }

    sections.into()
}

fn shortcut_row<'a>(shortcut: &Shortcut) -> Element<'a, Message> {
    let mut keys = row![].spacing(4).align_y(Alignment::Center);
    for (i, key) in shortcut.keys().into_iter().enumerate() {
        if i > 0 {
            keys = keys.push(text::caption("+"));
        }
        keys = keys.push(keycap(key));
    }

    settings::item_row(vec![
        column![
            text::body(shortcut.name),
            text::caption(shortcut.description),
        ]
        .spacing(2)
        .width(Length::Fill)
        .into(),
        keys.into(),
    ])
    .align_y(Alignment::Center)
    .into()
}

fn keycap<'a>(label: String) -> Element<'a, Message> {
    container(text::caption_heading(label))
        .padding([2, 8])
        .class(cosmic::theme::Container::custom(|theme| {
            let c = theme.cosmic();
            container::Style {
                background: Some(cosmic::iced::Background::Color(
                    c.background(false).component.base.into(),
                )),
                border: cosmic::iced::Border {
                    color: c.background(false).component.divider.into(),
                    width: 1.0,
                    radius: c.corner_radii.radius_xs.into(),
                },
                ..Default::default()
            }
        }))
        .into()
}
