-- Streaming services the user subscribes to, by TMDB provider ID. Name and
-- logo are a snapshot from TMDB's provider list, taken when the choice was
-- saved, so the choice stays readable when TMDB is down. Times are Unix seconds.
CREATE TABLE my_services (
    provider_id INTEGER NOT NULL PRIMARY KEY,
    name        TEXT    NOT NULL,
    logo_url    TEXT,
    added_at    INTEGER NOT NULL
) STRICT;
