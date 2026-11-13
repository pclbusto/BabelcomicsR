-- Criterio de ordenamiento para la vista de volúmenes/series.
ALTER TABLE setups ADD COLUMN volume_sort_order TEXT NOT NULL DEFAULT 'nombre_asc';
