-- Tasks to queue, written in the main database transaction whose changes call for them, then
-- moved to the tasks database once that transaction commits (the two databases share no
-- transaction). Rows left by a crash are moved at the next start.
CREATE TABLE IF NOT EXISTS ERA_TASK_HANDOVER
(
    SEQ         INTEGER PRIMARY KEY AUTOINCREMENT,
    ID          TEXT    NOT NULL,
    PRIORITY    INTEGER NOT NULL,
    GROUP_ID    TEXT,
    CLASS       TEXT    NOT NULL,
    SIMPLE_TYPE TEXT    NOT NULL,
    PAYLOAD     TEXT    NOT NULL
);
