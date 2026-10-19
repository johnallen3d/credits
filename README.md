# credits

Search TMDB for a movie or TV show and open its detail page. Rust (Axum + Tokio),
Askama templates, HTMX (vendored in `static/`, no Node build).

## Run

```sh
doppler secrets set TMDB_API_TOKEN --project credits --config dev   # TMDB "API Read Access Token"
mise run dev                                                        # http://127.0.0.1:3000
```

The server refuses to start without a real token, and only binds to loopback
(`CREDITS_BIND`, default `127.0.0.1:3000`) because it has no authentication.

## Routes

| HTML | JSON |
|------|------|
| `/?q=…` (full page, or results fragment for HTMX) | `/api/search?q=…` |
| `/movie/{id}` | `/api/movie/{id}` |
| `/tv/{id}` | `/api/tv/{id}` |

Both sides use `Catalog` (`src/catalog.rs`) and the types in `src/model.rs`.

## Test

```sh
mise run test   # uses a local fake TMDB; no token needed
```

This product uses the TMDB API but is not endorsed or certified by TMDB.
