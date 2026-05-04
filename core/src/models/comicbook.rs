use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Comicbook {
    pub id_comicbook: i64,
    pub path: String,
    pub id_comicbook_info: Option<i64>,
    pub calidad: Option<String>,
    pub en_papelera: bool,
    pub embedding: Option<String>, // JSON vector
    pub error_ultimo_escaneo: Option<String>,
    pub procesado: bool,
    pub catalog_match_similarity: Option<f64>,
    pub catalog_best_similarity: Option<f64>,
    pub catalog_selected_rank: Option<i64>,
    pub catalog_match_method: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewComicbook {
    pub path: String,
    pub id_comicbook_info: Option<i64>,
    pub calidad: Option<String>,
}

/// Vista enriquecida que combina el archivo físico con sus metadatos
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComicbookView {
    pub id_comicbook: i64,
    pub path: String,
    pub en_papelera: bool,
    pub calidad: Option<String>,
    pub error_ultimo_escaneo: Option<String>,
    pub procesado: bool,
    // De ComicbookInfo (puede ser None si no está catalogado)
    pub titulo: Option<String>,
    pub numero: Option<String>,
    pub calificacion: Option<f64>,
    // De Volume
    pub nombre_volume: Option<String>,
    // De Publisher
    pub nombre_publisher: Option<String>,
    // Ruta local de la portada
    pub ruta_cover: Option<String>,
    pub catalog_match_similarity: Option<f64>,
    pub catalog_best_similarity: Option<f64>,
    pub catalog_selected_rank: Option<i64>,
    pub catalog_match_method: Option<String>,
}
/// Filtros para la búsqueda de comics
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ComicFilter {
    pub query: Option<String>,
    pub clasificado: Option<bool>,
    pub min_calidad: Option<i32>,
}

/// Resultado de parsear la cadena de búsqueda con operadores `+` / `-`.
///
/// Sintaxis:
/// - `batman robin`           → incluir "batman" Y "robin"  (separador: espacio)
/// - `batman + robin`         → idéntico (+ explícito)
/// - `hulk -thor`             → incluir "hulk", excluir "thor"
/// - `invincible + iron man`  → incluir "invincible", "iron", "man"
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedQuery {
    pub must_include: Vec<String>,
    pub must_exclude: Vec<String>,
}

impl ParsedQuery {
    pub fn is_empty(&self) -> bool {
        self.must_include.is_empty() && self.must_exclude.is_empty()
    }
}

/// Parsea la cadena de búsqueda del usuario en tokens AND / NOT.
///
/// Tokeniza separando por `+`, `-` y espacios.
/// Un token precedido por `-` se convierte en exclusión; el resto en inclusión.
pub fn parse_search_query(raw: &str) -> ParsedQuery {
    let mut must_include = Vec::new();
    let mut must_exclude = Vec::new();

    // Dividir en segmentos usando `+` y `-` como separadores adicionales.
    // Conservamos si el segmento estaba precedido por `-`.
    let mut exclude_next = false;

    for part in raw.split_whitespace() {
        // Quitar `+` y `-` del principio del token
        let (prefix_minus, token) = if let Some(t) = part.strip_prefix('-') {
            (true, t)
        } else if let Some(t) = part.strip_prefix('+') {
            (false, t)
        } else {
            (exclude_next, part)
        };

        // Actualizar flag para el siguiente token si este era solo el operador
        if part == "-" {
            exclude_next = true;
            continue;
        } else if part == "+" {
            exclude_next = false;
            continue;
        }
        exclude_next = false;

        let word = token.trim().to_lowercase();
        if word.is_empty() {
            continue;
        }

        if prefix_minus {
            must_exclude.push(word);
        } else {
            must_include.push(word);
        }
    }

    ParsedQuery {
        must_include,
        must_exclude,
    }
}
