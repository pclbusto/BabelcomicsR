//! Única fuente de verdad de los atajos de teclado: de esta tabla salen tanto
//! el despacho de teclas como el panel de ayuda.

use cosmic::iced::keyboard::{Key, Modifiers, key::Named, key::Physical};
use cosmic::widget::menu::KeyBind;
use cosmic::widget::menu::key_bind::Modifier;
use cosmic_reader::options::Fit;

use crate::Message;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Category {
    Navegacion,
    Vista,
    Editor,
    Ayuda,
}

impl Category {
    pub const ALL: [Category; 4] = [
        Category::Navegacion,
        Category::Vista,
        Category::Editor,
        Category::Ayuda,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::Navegacion => "Navegación",
            Category::Vista => "Vista",
            Category::Editor => "Editor de viñetas",
            Category::Ayuda => "Ayuda",
        }
    }
}

pub(crate) struct Shortcut {
    pub bind: KeyBind,
    pub name: &'static str,
    pub description: &'static str,
    pub category: Category,
    /// `None` cuando la tecla la gestiona directamente COSMIC (p. ej. Esc).
    pub action: Option<Message>,
}

impl Shortcut {
    /// Partes a dibujar como teclas, p. ej. `["Ctrl", "S"]`.
    pub fn keys(&self) -> Vec<String> {
        let mut parts: Vec<String> = self
            .bind
            .modifiers
            .iter()
            .map(|m| {
                match m {
                    Modifier::Super => "Super",
                    Modifier::Ctrl => "Ctrl",
                    Modifier::Alt => "Alt",
                    Modifier::Shift => "Shift",
                }
                .to_string()
            })
            .collect();
        parts.push(match &self.bind.key {
            Key::Character(c) if c.as_str() == " " => "Espacio".into(),
            Key::Character(c) => c.to_uppercase(),
            Key::Named(Named::Escape) => "Esc".into(),
            Key::Named(Named::ArrowLeft) => "←".into(),
            Key::Named(Named::ArrowRight) => "→".into(),
            Key::Named(Named::Delete) => "Supr".into(),
            Key::Named(named) => format!("{named:?}"),
            other => format!("{other:?}"),
        });
        parts
    }

    pub fn matches_query(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        query.is_empty()
            || self.name.to_lowercase().contains(&query)
            || self.description.to_lowercase().contains(&query)
            || self
                .keys()
                .join("+")
                .to_lowercase()
                .contains(&query.replace(' ', ""))
    }
}

fn bind(modifiers: &[Modifier], key: Key) -> KeyBind {
    KeyBind {
        modifiers: modifiers.to_vec(),
        key,
    }
}

fn ch(c: &str) -> Key {
    Key::Character(c.into())
}

fn named(key: Named) -> Key {
    Key::Named(key)
}

pub(crate) fn all() -> Vec<Shortcut> {
    use Modifier::Ctrl;
    vec![
        Shortcut {
            bind: bind(&[], named(Named::ArrowRight)),
            name: "Página siguiente",
            description: "Avanza una página o viñeta; en lectura manga retrocede",
            category: Category::Navegacion,
            action: Some(Message::ArrowRight),
        },
        Shortcut {
            bind: bind(&[], named(Named::ArrowLeft)),
            name: "Página anterior",
            description: "Retrocede una página o viñeta; en lectura manga avanza",
            category: Category::Navegacion,
            action: Some(Message::ArrowLeft),
        },
        Shortcut {
            bind: bind(&[Ctrl], ch(" ")),
            name: "Cuadrícula de páginas",
            description: "Muestra todas las páginas para saltar a cualquiera",
            category: Category::Navegacion,
            action: Some(Message::TogglePageGrid),
        },
        Shortcut {
            bind: bind(&[], named(Named::F9)),
            name: "Panel de miniaturas",
            description: "Muestra u oculta las miniaturas de las páginas a la izquierda",
            category: Category::Navegacion,
            action: Some(Message::ToggleSidebar),
        },
        Shortcut {
            bind: bind(&[], ch("1")),
            name: "Tamaño real",
            description: "Muestra la página con el tamaño real de la imagen",
            category: Category::Vista,
            action: Some(Message::SetFit(Fit::Original)),
        },
        Shortcut {
            bind: bind(&[], ch("2")),
            name: "Ajustar al alto",
            description: "La página ocupa el alto de la ventana",
            category: Category::Vista,
            action: Some(Message::SetFit(Fit::Height)),
        },
        Shortcut {
            bind: bind(&[], ch("3")),
            name: "Ajustar al ancho",
            description: "La página ocupa el ancho de la ventana",
            category: Category::Vista,
            action: Some(Message::SetFit(Fit::Width)),
        },
        Shortcut {
            bind: bind(&[], ch("0")),
            name: "Página completa",
            description: "La página entera entra en la ventana",
            category: Category::Vista,
            action: Some(Message::SetFit(Fit::Page)),
        },
        Shortcut {
            bind: bind(&[], ch("4")),
            name: "Pantalla completa",
            description: "Entra o sale de pantalla completa",
            category: Category::Vista,
            action: Some(Message::ToggleFullscreen),
        },
        Shortcut {
            bind: bind(&[], named(Named::F11)),
            name: "Pantalla completa",
            description: "Entra o sale de pantalla completa",
            category: Category::Vista,
            action: Some(Message::ToggleFullscreen),
        },
        Shortcut {
            bind: bind(&[], named(Named::Escape)),
            name: "Cerrar paneles / salir de pantalla completa",
            description: "Cierra este panel o el de Acerca de; si no hay ninguno, sale de pantalla completa",
            category: Category::Vista,
            action: None,
        },
        Shortcut {
            bind: bind(&[], named(Named::Tab)),
            name: "Lectura / editor",
            description: "Alterna entre la lectura y el editor de viñetas",
            category: Category::Editor,
            action: Some(Message::ToggleMode),
        },
        Shortcut {
            bind: bind(&[], named(Named::Delete)),
            name: "Borrar viñeta",
            description: "Elimina la viñeta seleccionada en el editor",
            category: Category::Editor,
            action: Some(Message::DeleteSelectedPanel),
        },
        Shortcut {
            bind: bind(&[Ctrl], ch("s")),
            name: "Guardar viñetas",
            description: "Guarda las viñetas junto al cómic (archivo .json)",
            category: Category::Editor,
            action: Some(Message::SavePanels),
        },
        Shortcut {
            bind: bind(&[Ctrl], named(Named::F1)),
            name: "Atajos de teclado",
            description: "Muestra u oculta este panel",
            category: Category::Ayuda,
            action: Some(Message::ToggleShortcuts),
        },
        Shortcut {
            bind: bind(&[], named(Named::F1)),
            name: "Acerca del Lector de Babelcomics",
            description: "Muestra la versión, autoría, licencia y enlaces del proyecto",
            category: Category::Ayuda,
            action: Some(Message::ToggleAbout),
        },
    ]
}

/// Mensaje asociado a la combinación pulsada, si la app la gestiona.
pub(crate) fn message_for(
    modifiers: Modifiers,
    key: &Key,
    physical_key: Option<&Physical>,
) -> Option<Message> {
    all()
        .into_iter()
        .find(|s| s.action.is_some() && s.bind.matches(modifiers, key, physical_key))
        .and_then(|s| s.action)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binds_are_unique() {
        let all = all();
        for (index, shortcut) in all.iter().enumerate() {
            for other in &all[index + 1..] {
                assert!(
                    shortcut.bind != other.bind,
                    "{} y {} usan la misma tecla",
                    shortcut.name,
                    other.name
                );
            }
        }
    }

    #[test]
    fn f1_and_ctrl_f1_are_distinct() {
        let f1 = Key::Named(Named::F1);
        assert!(matches!(
            message_for(Modifiers::empty(), &f1, None),
            Some(Message::ToggleAbout)
        ));
        assert!(matches!(
            message_for(Modifiers::CTRL, &f1, None),
            Some(Message::ToggleShortcuts)
        ));
        assert!(matches!(
            message_for(Modifiers::empty(), &ch("2"), None),
            Some(Message::SetFit(Fit::Height))
        ));
        assert!(matches!(
            message_for(Modifiers::CTRL, &ch(" "), None),
            Some(Message::TogglePageGrid)
        ));
    }
}
