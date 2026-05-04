ALTER TABLE comicbooks ADD COLUMN catalog_match_similarity REAL;
ALTER TABLE comicbooks ADD COLUMN catalog_best_similarity REAL;
ALTER TABLE comicbooks ADD COLUMN catalog_selected_rank INTEGER;
ALTER TABLE comicbooks ADD COLUMN catalog_match_method TEXT;

CREATE INDEX IF NOT EXISTS idx_comicbooks_catalog_match_metrics
    ON comicbooks(catalog_match_similarity, catalog_selected_rank)
    WHERE catalog_match_similarity IS NOT NULL;
