-- Marca si un volumen fue importado desde la API (bajada parcial) sin tener ficheros físicos.
-- local = 0 → volumen descubierto por escaneo de archivos (por defecto).
-- local = 1 → stub importado desde la editorial en Comic Vine.
ALTER TABLE volumens ADD COLUMN local INTEGER NOT NULL DEFAULT 0;
