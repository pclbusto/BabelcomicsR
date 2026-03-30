use serde::{Deserialize, Serialize};

/// Tipo de página según el estándar ComicInfo.xml
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(i64)]
pub enum TipoPagina {
    Story       = 0,
    FrontCover  = 1,
    BackCover   = 2,
    InnerCover  = 3,
    Advertisement = 4,
    Other       = 5,
}

impl TipoPagina {
    pub fn from_db(val: i64) -> Self {
        match val {
            1 => TipoPagina::FrontCover,
            2 => TipoPagina::BackCover,
            3 => TipoPagina::InnerCover,
            4 => TipoPagina::Advertisement,
            5 => TipoPagina::Other,
            _ => TipoPagina::Story,
        }
    }

    pub fn to_db(self) -> i64 {
        self as i64
    }

    /// Nombre legible para mostrar en la UI
    pub fn label(self) -> &'static str {
        match self {
            TipoPagina::Story        => "Historia",
            TipoPagina::FrontCover   => "Portada",
            TipoPagina::BackCover    => "Contraportada",
            TipoPagina::InnerCover   => "Portada interior",
            TipoPagina::Advertisement => "Publicidad",
            TipoPagina::Other        => "Otro",
        }
    }

    /// Parsea el string de ComicInfo.xml al enum
    pub fn from_comicinfo_str(s: &str) -> Self {
        match s {
            "FrontCover"    => TipoPagina::FrontCover,
            "BackCover"     => TipoPagina::BackCover,
            "InnerCover"    => TipoPagina::InnerCover,
            "Advertisement" => TipoPagina::Advertisement,
            "Story"         => TipoPagina::Story,
            _               => TipoPagina::Other,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ComicbookDetail {
    pub id_detail:     i64,
    pub comicbook_id:  i64,
    #[sqlx(rename = "indicePagina")]
    pub indice_pagina: i64,
    #[sqlx(rename = "ordenPagina")]
    pub orden_pagina:  i64,
    #[sqlx(rename = "tipoPagina")]
    pub tipo_pagina:   i64,
    pub nombre_pagina: Option<String>,
}

impl ComicbookDetail {
    pub fn tipo(&self) -> TipoPagina {
        TipoPagina::from_db(self.tipo_pagina)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewComicbookDetail {
    pub comicbook_id:  i64,
    pub indice_pagina: i64,
    pub orden_pagina:  i64,
    pub tipo_pagina:   TipoPagina,
    pub nombre_pagina: Option<String>,
}
