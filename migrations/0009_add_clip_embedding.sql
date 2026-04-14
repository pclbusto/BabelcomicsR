-- Embedding visual CLIP para búsqueda de portadas similares.
-- 512 × f32 = 2048 bytes en little-endian.

-- Cache del embedding de la portada extraída del archivo CBZ/CBR.
-- Se calcula una vez y se reutiliza en cada búsqueda.
-- ALTER TABLE comicbooks ADD COLUMN clip_embedding BLOB;

-- Índice visual de portadas descargadas desde ComicVine.
-- La búsqueda compara el embedding del CBZ contra esta tabla.
-- ALTER TABLE comicbooks_info_covers ADD COLUMN clip_embedding BLOB;

-- Acelera la consulta "dame todas las portadas que YA tienen embedding"
CREATE INDEX IF NOT EXISTS idx_covers_clip_embedding
    ON comicbooks_info_covers(id_comicbook_info)
    WHERE clip_embedding IS NOT NULL;
