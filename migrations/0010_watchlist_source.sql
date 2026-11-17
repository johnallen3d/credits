-- Who or what recommended a To watch title, free text. watchlist_sources
-- remembers every source used, so it can be offered again after its titles
-- leave the list; names match case-insensitively and keep their first
-- spelling. Times are Unix seconds.
ALTER TABLE watchlist ADD COLUMN source TEXT;

CREATE TABLE watchlist_sources (
    name    TEXT    NOT NULL PRIMARY KEY COLLATE NOCASE,
    used_at INTEGER NOT NULL
) STRICT;
