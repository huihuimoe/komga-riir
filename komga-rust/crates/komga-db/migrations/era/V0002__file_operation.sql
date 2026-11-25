-- Files written next to or over library files, recorded before they are written. Sizes and
-- modification times (nanoseconds since the Unix epoch) identify the versions of the source and of
-- the verified output. REPLACED_URL is a file the output supersedes. COMMITTED is set with the
-- database changes that use the output.
-- IF NOT EXISTS: databases from before this migration track already have the table.
CREATE TABLE IF NOT EXISTS ERA_FILE_OPERATION
(
    ID              INTEGER PRIMARY KEY AUTOINCREMENT,
    SOURCE_URL      TEXT    NOT NULL,
    SOURCE_SIZE     INTEGER NOT NULL,
    SOURCE_MODIFIED INTEGER NOT NULL,
    TARGET_URL      TEXT    NOT NULL,
    REPLACED_URL    TEXT,
    REMOVE_SOURCE   INTEGER NOT NULL,
    OUTPUT_SIZE     INTEGER,
    OUTPUT_MODIFIED INTEGER,
    COMMITTED       INTEGER NOT NULL DEFAULT 0
);
