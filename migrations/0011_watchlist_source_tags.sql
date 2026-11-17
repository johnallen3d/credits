-- To watch sources become tags: a title can carry several, each one word
-- shown as #name. watchlist_title_sources holds a title's tags in the order
-- added (rowid); watchlist_sources still remembers every tag ever used.
-- Earlier free-text sources keep their words, joined by hyphens, and lose any
-- leading '#'; names that then match but for case merge into one.
CREATE TABLE watchlist_title_sources (
    kind TEXT    NOT NULL CHECK (kind IN ('movie', 'tv')),
    id   INTEGER NOT NULL,
    name TEXT    NOT NULL COLLATE NOCASE,
    PRIMARY KEY (kind, id, name)
) STRICT;

CREATE TABLE watchlist_sources_tags (
    name    TEXT    NOT NULL PRIMARY KEY COLLATE NOCASE,
    used_at INTEGER NOT NULL
) STRICT;

INSERT OR IGNORE INTO watchlist_sources_tags (name, used_at)
SELECT replace(ltrim(name, '#'), ' ', '-'), used_at
FROM watchlist_sources
WHERE ltrim(name, '#') <> ''
ORDER BY used_at DESC;

INSERT OR IGNORE INTO watchlist_title_sources (kind, id, name)
SELECT w.kind, w.id, t.name
FROM watchlist w
JOIN watchlist_sources_tags t ON t.name = replace(ltrim(w.source, '#'), ' ', '-')
WHERE w.source IS NOT NULL;

DROP TABLE watchlist_sources;
ALTER TABLE watchlist_sources_tags RENAME TO watchlist_sources;
ALTER TABLE watchlist DROP COLUMN source;
