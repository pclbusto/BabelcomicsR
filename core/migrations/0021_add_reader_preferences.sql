-- Preferencias del lector externo (cosmic-reader).
ALTER TABLE setups ADD COLUMN reader_path TEXT;
ALTER TABLE setups ADD COLUMN reader_fit TEXT NOT NULL DEFAULT 'page';
ALTER TABLE setups ADD COLUMN reader_direction TEXT NOT NULL DEFAULT 'ltr';
ALTER TABLE setups ADD COLUMN reader_guided TEXT NOT NULL DEFAULT 'ask';
