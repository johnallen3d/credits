-- Title and person pages opened in the web UI, for the home page. Display fields
-- are a snapshot from the latest visit. Rows are replaced on each visit, so a
-- larger rowid breaks ties within a second. Times are Unix seconds.
CREATE TABLE recently_viewed (
    kind      TEXT    NOT NULL CHECK (kind IN ('movie', 'tv', 'person')),
    id        INTEGER NOT NULL,
    name      TEXT    NOT NULL,
    year      TEXT,
    image_url TEXT,
    viewed_at INTEGER NOT NULL,
    PRIMARY KEY (kind, id)
) STRICT;
