-- Favorites are independent of the To watch list; keep display snapshots offline.
CREATE TABLE favorites (
    kind       TEXT    NOT NULL CHECK (kind IN ('movie', 'tv')),
    id         INTEGER NOT NULL,
    title      TEXT    NOT NULL,
    year       TEXT,
    poster_url TEXT,
    added_at   INTEGER NOT NULL,
    PRIMARY KEY (kind, id)
) STRICT;
