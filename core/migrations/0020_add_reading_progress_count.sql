ALTER TABLE comicbook_reading_progress
ADD COLUMN visited_pages_count INTEGER NOT NULL DEFAULT 0;

UPDATE comicbook_reading_progress
SET visited_pages_count = json_array_length(visited_pages_json);

