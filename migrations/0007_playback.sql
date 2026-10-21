-- Episodes and movies marked watched from what played on an Apple TV, as
-- reported by Home Assistant. Times are Unix seconds.

-- Who marked an episode: 'manual' (the web UI) or 'apple_tv'.
ALTER TABLE watched_episodes ADD COLUMN source TEXT NOT NULL DEFAULT 'manual';

-- Movies marked watched, by the web UI ('manual') or from the Apple TV.
CREATE TABLE watched_movies (
    movie_id   INTEGER NOT NULL PRIMARY KEY,
    watched_at INTEGER NOT NULL,
    source     TEXT    NOT NULL DEFAULT 'manual'
) STRICT;

-- One row per episode or movie seen playing on a player, by the app's own
-- key for it (Infuse: the full title; TV app: the content ID). kind:
-- 'episode' or 'movie'; a movie's season and episode are 0 and its show_key
-- and show_title are the movie's. Playing time adds up across sessions;
-- max_position is the furthest point reached, in seconds, when the live feed
-- reported positions. status: 'watching' (below the threshold, or waiting to
-- be resolved), 'marked', 'needs_confirmation', 'no_match' or 'ignored'.
-- tmdb_id: the TMDB show or movie it was matched to.
CREATE TABLE playback_progress (
    player          TEXT    NOT NULL,
    app             TEXT    NOT NULL,
    item_key        TEXT    NOT NULL,
    kind            TEXT    NOT NULL DEFAULT 'episode',
    show_key        TEXT    NOT NULL,
    show_title      TEXT    NOT NULL,
    title           TEXT    NOT NULL,
    season          INTEGER NOT NULL,
    episode         INTEGER NOT NULL,
    duration        INTEGER,
    playing_seconds INTEGER NOT NULL DEFAULT 0,
    max_position    INTEGER,
    first_seen      INTEGER NOT NULL,
    last_seen       INTEGER NOT NULL,
    status          TEXT    NOT NULL DEFAULT 'watching',
    tmdb_id         INTEGER,
    note            TEXT,
    PRIMARY KEY (player, app, item_key)
) STRICT;

-- Which TMDB show (kind 'episode') or movie (kind 'movie') an app's title
-- is. tmdb_id NULL: ignore it. decided_by: 'auto' (one unambiguous TMDB
-- match) or 'user'.
CREATE TABLE playback_mappings (
    app        TEXT    NOT NULL,
    kind       TEXT    NOT NULL,
    show_key   TEXT    NOT NULL,
    tmdb_id    INTEGER,
    decided_by TEXT    NOT NULL,
    decided_at INTEGER NOT NULL,
    PRIMARY KEY (app, kind, show_key)
) STRICT;

-- The last history row read per player, as JSON. Its playing time is counted
-- once the next row shows how long it lasted.
CREATE TABLE playback_cursor (
    player TEXT NOT NULL PRIMARY KEY,
    event  TEXT NOT NULL
) STRICT;
