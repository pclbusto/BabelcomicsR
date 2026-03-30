CREATE TABLE IF NOT EXISTS comicbooks_detail (
    id_detail     INTEGER PRIMARY KEY AUTOINCREMENT,
    comicbook_id  INTEGER NOT NULL REFERENCES comicbooks(id_comicbook) ON DELETE CASCADE,
    indicePagina  INTEGER NOT NULL DEFAULT 0,
    ordenPagina   INTEGER NOT NULL DEFAULT 0,
    tipoPagina    INTEGER NOT NULL DEFAULT 0,
    nombre_pagina TEXT,
    UNIQUE(comicbook_id, indicePagina)
);

CREATE INDEX IF NOT EXISTS idx_comicbooks_detail_comicbook ON comicbooks_detail(comicbook_id);
