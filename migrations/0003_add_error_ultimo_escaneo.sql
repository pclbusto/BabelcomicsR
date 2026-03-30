-- Añadir columna para registrar errores de escaneo/extracción
ALTER TABLE comicbooks ADD COLUMN error_ultimo_escaneo TEXT;
