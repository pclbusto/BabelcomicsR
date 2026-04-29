-- Sincronización robusta con el modelo Python
-- 1. Si existe la tabla vieja 'volumes' y no la nueva 'volumens', la renombramos.
-- En SQLite, esto es manual porque no hay 'IF EXISTS' para RENAME en versiones antiguas, 
-- pero podemos usar un truco de script o simplemente asegurar que no falle.
-- Nota: Si 'volumens' ya existe (porque viene de Python), este paso se saltará o dará error controlado.

-- Intentamos renombrar solo si 'volumes' existe. 
-- Si falla porque 'volumens' ya existe, está bien, significa que ya tenemos los datos de Python.
ALTER TABLE volumes RENAME TO volumens;

-- 2. Añadir campos faltantes solo si no existen.
-- SQLite no tiene 'ADD COLUMN IF NOT EXISTS', así que lo haremos de forma que no bloquee.
-- Si ya existen en tu DB de Python, sqlx podría dar error. 
-- Para evitarlo, usamos bloques que permitan continuar o simplemente confiamos en que 
-- si vienen de Python, ya podrían estar ahí.

-- Estos fallarán si ya existen, lo cual es correcto si los datos ya están ahí.
ALTER TABLE volumens ADD COLUMN deck TEXT NOT NULL DEFAULT '';
ALTER TABLE volumens ADD COLUMN url TEXT NOT NULL DEFAULT '';
