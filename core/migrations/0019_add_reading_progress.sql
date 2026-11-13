CREATE TABLE IF NOT EXISTS comicbook_reading_progress (
    comicbook_id             INTEGER PRIMARY KEY
        REFERENCES comicbooks(id_comicbook) ON DELETE CASCADE,
    source_path              TEXT NOT NULL,
    source_size_bytes        INTEGER NOT NULL DEFAULT 0,
    source_modified_unix_ms  INTEGER NOT NULL DEFAULT 0,
    total_pages              INTEGER NOT NULL DEFAULT 0,
    last_page_index          INTEGER,
    visited_pages_json       TEXT NOT NULL DEFAULT '[]',
    completed_at             TEXT,
    updated_at               TEXT NOT NULL
);
