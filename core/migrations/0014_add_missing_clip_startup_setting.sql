-- Migration 0010 was published without executing its ALTER TABLE statement.
-- Add the missing setting in a new migration so databases that already
-- recorded migration 0010 can be repaired without changing its checksum.
ALTER TABLE setups ADD COLUMN clip_al_arranque BOOLEAN NOT NULL DEFAULT 1;
