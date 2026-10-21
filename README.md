# credits

A personal movie and TV browser: search TMDB, open a title or person, see
ratings, where to watch, and what you've watched.

- **Stack:** Rust (Axum + Tokio), Askama templates, HTMX (vendored in
  `static/`, no Node build), SQLite (SQLx)
- **Data:** TMDB (titles, people, offers), OMDb (ratings), Home Assistant
  (optional, Apple TV playback)
- **Every page has a JSON twin** under `/api/…` (see [Routes](#routes)),
  described by the OpenAPI 3.1 spec in [`openapi.json`](openapi.json),
  also served at `/api/openapi.json`
- **Two binaries:** `credits-web` is the server; `credits` is a terminal
  client for a running server (see [Terminal client](#terminal-client))

## Run

```sh
doppler secrets set TMDB_API_TOKEN --project credits --config dev   # TMDB "API Read Access Token"
doppler secrets set OMDB_API_KEY --project credits --config dev     # optional, enables ratings
mise run dev                                                        # credits-web on http://127.0.0.1:3000
mise run tui                                                        # credits, the terminal client
```

- Refuses to start without a real TMDB token
- No authentication, so it only binds to loopback

### Configuration

| Variable | Default | Purpose |
|----------|---------|---------|
| `TMDB_API_TOKEN` | required | TMDB API Read Access Token |
| `OMDB_API_KEY` | unset | Ratings; without it ratings show as unavailable |
| `CREDITS_BIND` | `127.0.0.1:3000` | Listen address |
| `CREDITS_DB` | `./credits.db` | SQLite database (cache and your lists) |
| `CREDITS_REGION` | `US` | Two-letter country for where to watch and My services |
| `HA_URL`, `HA_TOKEN` | unset | Home Assistant URL and long-lived token; enables Seen on Apple TV |
| `CREDITS_HA_PLAYERS` | `media_player.living_room_appletv` | Comma-separated `media_player` entities to follow |

These configure `credits-web`. The terminal client reads only a TOML file:

```toml
# ~/.config/credits/config.toml ($XDG_CONFIG_HOME/credits, or $CREDITS_CONFIG)
url = "https://credits.jallen7usa.com"
theme = "rose-pine"   # optional: rose-pine (default), tokyo-night, catppuccin, classic
```

## Features

### Search and browsing

- Search movies, TV shows and people from one box (HTMX, live results)
- Keyboard navigation: ↑/↓ move through results (or Recently viewed when
  the box is empty), Enter opens the highlighted item
- Home page shows the 12 most recently viewed titles and people
- Movie and TV pages: poster, key crew (directors and writers; for TV,
  creators and most frequent directors), top-billed cast, season list
- Full credits pages with the whole cast and crew by department
- Season pages with episode list, air dates and IMDb episode scores
- Episode pages with guest stars, series regulars credited on it,
  directors, writers, crew by department, and previous/next links
- Person pages:
  - **Known for** row of up to eight titles, ranked from combined credits
    (talk shows and self appearances left out)
  - Filmography as department tabs (main department first) with poster
    thumbnails
  - Filter All / Movies / TV and sort by date or rating (`?sort=rating`);
    tab, kind and sort live in the URL
- Responsive layout down to phone width

### Themes

- Rosé Pine (default), Tokyo Night (Night), Catppuccin (Mocha), and
  Classic (the original light/dark look that follows the system)
- Web: Theme picker in the footer, remembered per browser (local
  storage) and applied before the page paints; without JavaScript the
  picker is hidden and Rosé Pine shows
- Terminal client: `theme` in `config.toml`; Classic uses the terminal's
  own colors, the others paint their background (truecolor); an unknown
  name stops startup with the valid choices

### Ratings

- One preferred score per title: Rotten Tomatoes for movies, IMDb for TV
- Other scores listed beneath, with source icons (fresh tomato at 60%+,
  splat below)
- A missing preferred score is shown as missing, never substituted
- Looked up on OMDb by the IMDb ID TMDB reports, never by title
- Rating sort uses the Tomatometer, else IMDb as a percentage; unscored
  titles follow in date order
  - First rating sort of a big filmography costs one TMDB + one OMDb
    request per title, eight at a time

### Where to watch

- Offers for `CREDITS_REGION`: subscription, free, free with ads, rent,
  buy, in TMDB's order (JustWatch data via TMDB)
- Collapsed by default to one line counting each kind
- Services link to TMDB's offers page, which links to each service
- Comes in the same TMDB request and cache entry as the title

### My services

- Pick the streaming services you subscribe to (linked in the footer);
  saved on each change
- Options from TMDB's movie and TV service lists for the region, grouped:
  yours, popular, everything else (each A to Z)
  - Popular is a fixed list of major US services, or TMDB's top ten
    elsewhere
  - Add-on channels (Amazon Channel, Roku Premium Channel, Apple TV
    channel) hidden unless already chosen
- Title pages highlight your services among the offers; the collapsed
  summary names the ones that include it (rent and buy excluded)
- To watch shows a badge per title for each of your services, and an
  **On my services** filter (`?services=mine`)
- Nothing is highlighted or filtered until you choose a service

### To watch and Favorites

- Bookmark (To watch) and heart (Favorites) toggles on title pages,
  search results, filmography rows and both lists
- Both lists filter All / Movies / TV and sort by Added (newest first) or
  Rating
- Each entry snapshots name, year and poster when added
- Favoriting a TV show marks its currently aired seasons watched
  (including specials); future and empty seasons stay unwatched
- Unhearting only removes the favorite, keeping watched history

### Watched and spoilers

- Mark a movie, show, season or episode watched
- Episode summaries and IMDb episode scores stay blurred until that
  episode, its season or the show is watched; select one to peek
- If watched marks can't be read, spoilers stay hidden
- TV, season and episode JSON carry the show's `watched` marks, with
  `state` `read` or `unreadable`

### Seen on Apple TV

Optional; on when `HA_URL` and `HA_TOKEN` are set. credits only connects
out to Home Assistant.

- Follows the players live over Home Assistant's WebSocket API, and on
  each (re)connect catches up from the REST history where it left off
  (up to the recorder's 10 days)
- Reconnects with backoff from 5 seconds up to 5 minutes
- Understands:
  - **Infuse** episodes ("New Girl - S4 ∙ E8 - Teachers") and movies
    (any other title, matched by TMDB search, year and runtime)
  - **Apple TV app** episodes from the content ID (`A0178901003` is S1 E3,
    checked against TMDB's runtime) and feature-length videos (40 min+)
    as movies
  - Other apps (YouTube, Music, ESPN, trailers) are ignored and only
    totalled
- Marks watched when:
  - Episodes: 90% played or reached, or 80% once a later episode starts
  - Movies: 90%, or all but the last 10 minutes (at least 80%)
- Play time adds up across sessions
- Shows and movies with one fitting TMDB match are matched automatically;
  others wait on `/playback` (footer) for a choice, a TMDB ID, or ignore
- Each item is marked once; unmarking by hand sticks
- Auto marks are labelled "Watched on Apple TV"; `/playback` and About
  show the live connection and last history read

### Caching and degraded states

- TMDB title, season and person responses and OMDb ratings are cached in
  SQLite for 24 hours; search is not cached
- When a provider fails after that, the expired copy is shown and marked
  stale with its fetch date; "not found" and token errors are never masked
- Missing OMDb key or IMDb ID: ratings show as unavailable, page still
  renders
- A title whose offers can't be loaded is marked "Availability unknown"
  and left out of On my services
- If saved lists, favorites or services can't be read, pages say so
  rather than showing them empty, and saving is refused
- JSON carries the same state: `state` (`fresh` / `stale` /
  `unavailable`) and `fetched_at` on `metadata`, `ratings`,
  `where_to_watch` and `/api/services`

### Terminal client

- `credits` browses a running server over its `/api/…` JSON; no database,
  TMDB token or Doppler needed
- Live search (debounced) for movies, TV shows and people
- Movie and TV pages: facts, ratings, overview, where to watch (your
  services starred), key crew and top cast
- TV pages list seasons: number, name, episode count, year, watched mark
- Season pages: episodes with air date, runtime, IMDb score and watched
  mark
- Episode pages: summary, directors and writers, guest stars and series
  regulars credited on it
- Unwatched episodes' summaries and scores stay hidden, as on the web;
  `s` peeks the highlighted (or open) episode
- Person pages: born/died, first paragraph of the biography, known for,
  filmography by department
- To watch (`w`) and Favorites (`f`) list screens, read-only: year, kind,
  title, score when sorted by rating, and your services on To watch
- Lists filter and sort like the web page: `t` cycles All / Movies / TV,
  `o` toggles Added / Rating, `m` toggles On my services (To watch, once
  services are chosen)
- Keys: type to search, ↓/Enter into results, Esc leaves the box, j/k or
  ↑/↓ move, Enter/l open, Esc/Backspace/h back, `/` new search, `w`/`f`
  lists, `r` reload, `s` peek, `[`/`]` previous/next episode, `q` or
  Ctrl-C quit
- Shows the server's degraded states: stale details, ratings, episode
  scores and offers with their date, ratings unavailable, availability
  unknown, and watched marks unreadable (spoilers stay hidden)
- Lists: unreadable lists or services shown as errors (never empty),
  titles of unknown availability, stale listings and ratings
- Server unreachable, not found or provider errors are shown in place;
  `r` (or Enter on a failed search) retries
- Colors follow the `theme` setting (see [Themes](#themes))
- Not yet: toggling To watch, Favorite and Watched; playback

## Routes

| HTML | JSON |
|------|------|
| `/?q=…` (full page, or results fragment for HTMX) | `/api/search?q=…` |
| `/movie/{id}`, `/tv/{id}` | `/api/movie/{id}`, `/api/tv/{id}` |
| `/movie/{id}/credits`, `/tv/{id}/credits` | `/api/movie/{id}/credits`, `/api/tv/{id}/credits` |
| `/tv/{id}/season/{n}` | `/api/tv/{id}/season/{n}` |
| `/tv/{id}/season/{n}/episode/{e}` | `/api/tv/{id}/season/{n}/episode/{e}` |
| `/person/{id}` (`?sort=rating`) | `/api/person/{id}` (`?sort=rating`) |
| `/watchlist` (`?kind=movie\|tv`, `?sort=rating`, `?services=mine`) | `/api/watchlist` (same parameters) |
| `/favorites` (`?kind=movie\|tv`, `?sort=rating`) | `/api/favorites` (same parameters) |
| `/services` | `/api/services` |
| `/playback` | `/api/playback` |
| `/about` | |
| | `/api/openapi.json` (this API's OpenAPI spec) |

Form posts:

| Route | Form |
|-------|------|
| `POST /movie/{id}/watchlist`, `POST /tv/{id}/watchlist` | `listed=true\|false` |
| `POST /movie/{id}/favorite`, `POST /tv/{id}/favorite` | `listed=true\|false` |
| `POST /movie/{id}/watched`, `POST /tv/{id}/watched` | `watched=true\|false` |
| `POST /tv/{id}/season/{n}/watched`, `POST /tv/{id}/season/{n}/episode/{e}/watched` | `watched=true\|false` |
| `POST /services` | one `service={provider id}` per chosen service |
| `POST /playback/decide` | `app`, `show`, `tv_id` or `other_id`, `action=use\|ignore` |

HTML and JSON both go through `Catalog` (`src/catalog.rs`) and the types in
`src/model.rs`.

### OpenAPI

- `openapi.json` is generated, never edited: each JSON handler's
  `#[utoipa::path]` both registers the route and describes it, and the
  response types derive `ToSchema` (doc comments become descriptions)
- Served at `/api/openapi.json`, built from the same routes at startup
- Regenerate with `mise run openapi`
- `mise run hooks` once per clone enables a pre-commit hook that
  regenerates it when `src/` or `Cargo.*` changes and stops the commit if
  it changed, so the spec is staged with the code
- The `openapi` test fails when the committed copy is out of date, so CI
  does too

## Develop

```sh
mise run test               # local fake TMDB and OMDb servers; no keys needed
mise run openapi            # regenerate openapi.json
mise run hooks              # enable the pre-commit hook (once per clone)
nix build .#default         # crane-built credits-web (mainProgram) and credits
nix flake check             # tests, as CI runs them
```

- CI (`.github/workflows/ci.yml`) runs `nix flake check` and
  `nix build .#default`, pushing to the `johnallen3d` Cachix cache

This product uses the TMDB API but is not endorsed or certified by TMDB.
Ratings via the [OMDb API](https://www.omdbapi.com/).
