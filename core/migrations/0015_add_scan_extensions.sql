CREATE TABLE IF NOT EXISTS setup_scan_extensions (
    setup_key TEXT NOT NULL,
    extension TEXT NOT NULL COLLATE NOCASE,
    PRIMARY KEY (setup_key, extension),
    FOREIGN KEY (setup_key) REFERENCES setups(setupkey) ON DELETE CASCADE
);

INSERT OR IGNORE INTO setup_scan_extensions (setup_key, extension) VALUES
    ('default', 'cbz'),
    ('default', 'cbr'),
    ('default', 'cb7'),
    ('default', 'zip'),
    ('default', 'rar'),
    ('default', '7z'),
    ('default', 'pdf');
