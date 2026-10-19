# credits

Search TMDB for a movie, TV show or person and open its detail page, with ratings from
OMDb. Rust (Axum + Tokio), Askama templates, HTMX (vendored in `static/`, no
Node build), SQLite (SQLx) for the provider cache.

## Run

```sh
doppler secrets set TMDB_API_TOKEN --project credits --config dev   # TMDB "API Read Access Token"
doppler secrets set OMDB_API_KEY --project credits --config dev     # optional, enables ratings
mise run dev                                                        # http://127.0.0.1:3000
```

The server refuses to start without a real token, and only binds to loopback
(`CREDITS_BIND`, default `127.0.0.1:3000`) because it has no authentication.

## Ratings

Detail pages show one preferred score: Rotten Tomatoes for movies, IMDb for TV
shows. Other supported scores appear beneath it, labeled by source; a missing
preferred score is shown as missing, never replaced. Ratings are looked up on
OMDb by the IMDb ID that TMDB reports, never by title.

OMDb responses are cached in SQLite (`CREDITS_DB`, default `./credits.db`) and
stay fresh for 24 hours. After that the next view refetches; if OMDb fails
(timeout, quota, outage) the cached scores are shown and marked stale with
their fetch date. Without `OMDB_API_KEY`, or without an IMDb ID, ratings are
reported as unavailable and the page still renders. The key never leaves the
server; JSON responses include each score's `source`, `scale`, the
`imdb_id`, `state` (`fresh` / `stale` / `unavailable`) and `fetched_at`.

## Routes

| HTML | JSON |
|------|------|
| `/?q=…` (full page, or results fragment for HTMX) | `/api/search?q=…` |
| `/movie/{id}` | `/api/movie/{id}` |
| `/tv/{id}` | `/api/tv/{id}` |
| `/movie/{id}/credits`, `/tv/{id}/credits` | `/api/movie/{id}/credits`, `/api/tv/{id}/credits` |
| `/tv/{id}/season/{n}` | `/api/tv/{id}/season/{n}` |
| `/person/{id}` | `/api/person/{id}` |

Both sides use `Catalog` (`src/catalog.rs`) and the types in `src/model.rs`.
Detail responses include a `ratings` object, key crew (directors and writers
for movies; creators and most frequent directors for TV), the top-billed cast,
cast/crew totals and, for TV, the season list. Credits responses carry the full
cast and the crew grouped by department. Person responses carry the
filmography grouped by department, the person's main department first.

## TMDB cache

Title, season and person responses from TMDB go through the same SQLite cache
as ratings and stay fresh for 24 hours, so a title page, its credits page and
their JSON counterparts share one upstream request. If TMDB fails after that,
the expired copy is served with a notice giving the date it was saved; "not
found" and token errors are never masked. JSON responses carry the same
information as `metadata` (`state`: `fresh` / `stale`, and `fetched_at`).
Search is not cached.

## Test

```sh
mise run test   # uses local fake TMDB and OMDb servers; no keys needed
```

This product uses the TMDB API but is not endorsed or certified by TMDB.
Ratings via the [OMDb API](https://www.omdbapi.com/).
