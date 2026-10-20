-- TV shows, seasons and episodes marked as watched in the web UI. Season pages
-- blur an episode's summary and rating as spoilers until it, its season or its
-- whole show is marked watched. Times are Unix seconds.
CREATE TABLE watched_shows (
    tv_id      INTEGER NOT NULL PRIMARY KEY,
    watched_at INTEGER NOT NULL
) STRICT;

CREATE TABLE watched_seasons (
    tv_id      INTEGER NOT NULL,
    season     INTEGER NOT NULL,
    watched_at INTEGER NOT NULL,
    PRIMARY KEY (tv_id, season)
) STRICT;

CREATE TABLE watched_episodes (
    tv_id      INTEGER NOT NULL,
    season     INTEGER NOT NULL,
    episode    INTEGER NOT NULL,
    watched_at INTEGER NOT NULL,
    PRIMARY KEY (tv_id, season, episode)
) STRICT;
