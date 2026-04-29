-- Publishers / Editoriales
CREATE TABLE IF NOT EXISTS publishers (
    id_publisher   INTEGER PRIMARY KEY AUTOINCREMENT,
    nombre         TEXT    NOT NULL,
    descripcion    TEXT,
    id_comicvine   INTEGER UNIQUE,
    image_url      TEXT
);

-- Volumes / Series
CREATE TABLE IF NOT EXISTS volumes (
    id_volume        INTEGER PRIMARY KEY AUTOINCREMENT,
    nombre           TEXT    NOT NULL,
    descripcion      TEXT,
    id_publisher     INTEGER REFERENCES publishers(id_publisher) ON DELETE SET NULL,
    anio_inicio      INTEGER,
    cantidad_numeros INTEGER,
    id_comicvine     INTEGER UNIQUE,
    image_url        TEXT
);

-- Comic info / Metadatos de números
CREATE TABLE IF NOT EXISTS comicbooks_info (
    id_comicbook_info INTEGER PRIMARY KEY AUTOINCREMENT,
    titulo            TEXT    NOT NULL,
    id_volume         INTEGER REFERENCES volumes(id_volume) ON DELETE SET NULL,
    numero            TEXT,
    resumen           TEXT,
    calificacion      REAL,
    id_comicvine      INTEGER UNIQUE,
    url_api_detalle   TEXT,
    fue_actualizado_api INTEGER NOT NULL DEFAULT 0
);

-- Portadas variantes de un número
CREATE TABLE IF NOT EXISTS comicbooks_info_covers (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    id_comicbook_info INTEGER NOT NULL REFERENCES comicbooks_info(id_comicbook_info) ON DELETE CASCADE,
    url_original      TEXT    NOT NULL,
    ruta_local        TEXT
);

-- Archivos físicos de comics
CREATE TABLE IF NOT EXISTS comicbooks (
    id_comicbook      INTEGER PRIMARY KEY AUTOINCREMENT,
    path              TEXT    NOT NULL UNIQUE,
    id_comicbook_info INTEGER REFERENCES comicbooks_info(id_comicbook_info) ON DELETE SET NULL,
    calidad           TEXT,
    en_papelera       INTEGER NOT NULL DEFAULT 0,
    embedding         TEXT    -- JSON vector (opcional, para AI)
);

-- Configuración de la app
CREATE TABLE IF NOT EXISTS setups (
    setupkey          TEXT    PRIMARY KEY,
    api_key_encrypted TEXT,
    modo_oscuro       INTEGER NOT NULL DEFAULT 0,
    thumbnail_size    INTEGER NOT NULL DEFAULT 200,
    items_por_pagina  INTEGER NOT NULL DEFAULT 50,
    num_workers       INTEGER NOT NULL DEFAULT 4,
    idioma            TEXT
);

-- Directorios de escaneo
CREATE TABLE IF NOT EXISTS setup_directorios (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    path      TEXT    NOT NULL UNIQUE,
    setup_key TEXT    NOT NULL REFERENCES setups(setupkey) ON DELETE CASCADE
);

-- Insertar config por defecto
INSERT OR IGNORE INTO setups (setupkey) VALUES ('default');

-- Índices para mejorar rendimiento
CREATE INDEX IF NOT EXISTS idx_comicbooks_info        ON comicbooks(id_comicbook_info);
CREATE INDEX IF NOT EXISTS idx_comicbooks_en_papelera ON comicbooks(en_papelera);
CREATE INDEX IF NOT EXISTS idx_volumes_publisher      ON volumes(id_publisher);
CREATE INDEX IF NOT EXISTS idx_comicbooks_info_volume ON comicbooks_info(id_volume);
