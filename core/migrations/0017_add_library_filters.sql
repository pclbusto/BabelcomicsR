-- Filtros compartidos por todos los frontends de Babelcomics.
ALTER TABLE setups ADD COLUMN catalog_filter TEXT NOT NULL DEFAULT 'all';
ALTER TABLE setups ADD COLUMN volume_update_filter TEXT NOT NULL DEFAULT 'all';

-- Migra la tabla privada usada por versiones anteriores de COSMIC. Se crea
-- vacía en instalaciones nuevas para que la migración sea válida en ambos casos.
CREATE TABLE IF NOT EXISTS cosmic_preferences (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

UPDATE setups
SET catalog_filter = COALESCE(
        (SELECT value FROM cosmic_preferences
         WHERE key = 'catalog_filter'
           AND value IN ('all', 'catalogued', 'uncatalogued')),
        catalog_filter
    ),
    volume_update_filter = COALESCE(
        (SELECT value FROM cosmic_preferences
         WHERE key = 'volume_update_filter'
           AND value IN ('all', 'updated', 'not_updated')),
        volume_update_filter
    )
WHERE setupkey = 'default';

DROP TABLE cosmic_preferences;
