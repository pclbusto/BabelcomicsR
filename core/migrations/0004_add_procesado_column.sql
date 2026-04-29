-- Añadir columna para evitar re-procesar cómics que ya se intentaron extraer (éxito o error)
ALTER TABLE comicbooks ADD COLUMN procesado INTEGER NOT NULL DEFAULT 0;
