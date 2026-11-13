-- Calcular embeddings CLIP de las portadas al terminar de bajar o actualizar un volumen.
ALTER TABLE setups ADD COLUMN clip_al_descargar INTEGER NOT NULL DEFAULT 1;
