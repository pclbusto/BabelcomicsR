-- Añadir campos para la configuración detallada de ComicVine
ALTER TABLE setups ADD COLUMN intervalo_api REAL DEFAULT 0.5;
ALTER TABLE setups ADD COLUMN api_url TEXT DEFAULT 'https://comicvine.gamespot.com/api/';
