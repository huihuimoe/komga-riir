-- Entities whose search index document is out of date, written in the transaction that changes
-- them. AUTOINCREMENT keeps a SEQ from being reused, so that a record replaced while it was being
-- applied is told apart from the one that was applied.
-- IF NOT EXISTS: databases from before this migration track already have the table.
CREATE TABLE IF NOT EXISTS ERA_SEARCH_INDEX_PENDING
(
    SEQ       INTEGER PRIMARY KEY AUTOINCREMENT,
    ENTITY    TEXT NOT NULL,
    ENTITY_ID TEXT NOT NULL,
    UNIQUE (ENTITY, ENTITY_ID)
);
