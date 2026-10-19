-- Normalized upstream responses keyed by provider and that provider's identifier
-- (e.g. provider 'omdb', key 'tt0133093'). Times are Unix seconds.
CREATE TABLE provider_cache (
    provider   TEXT    NOT NULL,
    key        TEXT    NOT NULL,
    payload    TEXT    NOT NULL,
    fetched_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    PRIMARY KEY (provider, key)
) STRICT;
