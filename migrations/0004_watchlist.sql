-- Movies and TV shows saved to the "To watch" list in the web UI. Display
-- fields are a snapshot from TMDB, refreshed whenever the title is added again;
-- added_at keeps the first addition. A larger rowid breaks ties within a
-- second. Times are Unix seconds.
CREATE TABLE watchlist (
    kind       TEXT    NOT NULL CHECK (kind IN ('movie', 'tv')),
    id         INTEGER NOT NULL,
    title      TEXT    NOT NULL,
    year       TEXT,
    poster_url TEXT,
    added_at   INTEGER NOT NULL,
    PRIMARY KEY (kind, id)
) STRICT;
