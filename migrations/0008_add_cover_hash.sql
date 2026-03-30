-- Almacena el hash perceptual (dHash) de la portada descargada para comparación de similitud
ALTER TABLE comicbooks_info_covers ADD COLUMN cover_hash TEXT;
