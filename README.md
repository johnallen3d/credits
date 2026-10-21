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

Person pages can sort each department's credits by rating (`?sort=rating`):
the Rotten Tomatoes Tomatometer where there is one, otherwise the IMDb score
as a percentage. Unscored credits follow in date order. Each title's IMDb ID
comes from its cached TMDB entry, so the first rating sort of a busy
filmography costs one TMDB and one OMDb request per title, eight at a time;
date order, the default, looks nothing up.

## Where to watch

Movie and TV pages list where the title can be watched in one country:
streaming with a subscription, free, free with ads, rent and buy, each with the
services in TMDB's order. The list starts collapsed to one line counting the
offers of each kind. The data is JustWatch's, via TMDB, and comes in the
same TMDB request and cache entry as the rest of the page. Set the country with
`CREDITS_REGION` (a two-letter code, default `US`). TMDB doesn't link straight to
each service, so the services link to TMDB's page of every offer for the
country, which does.

When TMDB data is stale, the offers are marked with the date they were listed.
Cache entries saved before this feature are refetched; if TMDB is down and only
such an entry exists, where to watch is reported as unavailable. JSON responses
carry `where_to_watch` with `region`, `state` (`fresh` / `stale` /
`unavailable`), `link`, `offers` and `fetched_at`.

## My services

Choose the streaming services you subscribe to on My services (linked from
the header). The choices come from TMDB's lists of movie and TV services for
`CREDITS_REGION` and are cached like other TMDB data. The page lists your
services first, then the popular ones, then the rest, each A to Z. Popular
is a fixed list of the major US subscription services (Netflix, Prime
Video, Disney Plus, HBO Max, Hulu, Apple TV, Paramount Plus, Peacock,
YouTube TV and ESPN Plus); in other regions it's TMDB's ten most popular.
Channels sold through another service (names ending in Amazon Channel, Roku
Premium Channel or Apple TV channel) aren't listed unless already chosen,
since TMDB nearly always lists the service itself alongside them.
The choice itself is saved in SQLite with each service's name and logo.
Movie and TV pages then highlight your services among the offers, and the
collapsed summary names those that include the title (streaming, free or
free with ads; renting and buying cost extra). The To watch list adds a
small badge per title for each of your services that includes it, and an
On my services filter (`?services=mine`) that combines with the kind and
sort. Each title's offers come from its cached TMDB entry, so the first
visit after the cache expires costs one TMDB request per title, eight at a
time. Favorites aren't filtered by service.

Nothing is highlighted or filtered until you choose a service. If TMDB is
down, expired offers and service lists are used and marked stale; a title
whose offers can't be loaded at all is marked "Availability unknown" and
left out of On my services, with a count above the list. If TMDB's list of
services can't be loaded, My services shows only your current choice, which
you can still remove. If your saved choice can't be read, pages say so
rather than treating it as empty, and saving is refused so it isn't
overwritten. JSON: `/api/services` carries `chosen`, `options` (each with
`chosen` and `group`: `mine`, `popular` or `more`), `state`,
`complete` and `fetched_at`; title offers carry `mine`
and `where_to_watch.my_services` (`not_chosen` / `chosen` / `unreadable`);
To watch titles carry `availability` and the list a `services` summary.

## Spoilers

Season and episode pages blur episode summaries and IMDb episode scores until
that episode, its season or the whole show is marked watched; select a blurred
item to peek at it.
Watched marks are stored in the same SQLite database and survive restarts. If
they can't be read, spoilers stay hidden.

## Seen on Apple TV

With Home Assistant set up, TV episodes played on the Apple TV are marked
watched without a click. Set `HA_URL` (e.g. `http://homeassistant:8123`) and
`HA_TOKEN` (a long-lived Home Assistant token for credits) in Doppler;
`CREDITS_HA_PLAYERS` lists the `media_player` entities to read, comma
separated (default `media_player.living_room_appletv`). Without the URL and
token the feature is off. credits only connects out to Home Assistant; it
still has no inbound port beyond the loopback web server.

credits follows the players live over Home Assistant's WebSocket API, which
also reports how far into the episode playback is. Each time it connects,
it first reads each player's history from the REST API, from where the last
read stopped (on the first run, the 10 days Home Assistant's recorder
keeps), so time when credits wasn't running or connected is caught up as
long as it starts at least every 10 days. If the connection drops it
retries, waiting 5 seconds and doubling up to 5 minutes, reading the
history on each attempt. Time spent playing adds up per episode across
sessions. Two apps are understood:

- Infuse titles episodes "New Girl - S4 ∙ E8 - Teachers".
- Apple's TV app reports the show's name and a content ID ending in the
  season (two digits) and episode (three): `A0178901003` is S1 E3. The
  played duration must also fit TMDB's runtime for that episode (4 minutes
  shorter to 8 longer), which checked out on every episode in the history.

Other apps (YouTube, Music, ESPN, movies) are ignored and only totalled on
the page. An episode counts as watched once 90% of its duration has played
or the live position has reached 90% (skipping ahead counts), or 80% once
a later episode of the same show has started, since players move on
at the end credits. If Infuse doesn't report the duration, TMDB's runtime is
used once the show is known. A row without a title continues the episode
before it when it reports the same duration, as Infuse does after resuming.

The show is matched to TMDB by exact name (ignoring case and punctuation).
If exactly one show of that name has the episode, with a fitting runtime,
it's used for the show's later episodes too. Otherwise the Seen on Apple TV
page (`/playback`, linked in the footer) asks which show it is, offering
TMDB's matches or a TMDB ID, or to ignore the show; the choice marks the
waiting episodes and applies to later ones. Each episode is marked once:
unmarking it by hand sticks. Auto marks are stored with `source` =
`apple_tv` and labelled "Watched on Apple TV" on season pages and on the
episode's page. The page shows recent episodes with how much was played,
whether the live feed is connected and when the history was last read; the
About page shows the connection too. If TMDB fails, matching is retried
with the next change or read.

## To watch

Movies and TV shows can be saved to a "To watch" list from their own page,
from search results, or from any row of a person's filmography, with the
bookmark toggle. The list
page (linked from the header) shows the newest additions first and filters
and sorts like a filmography: All / Movies / TV (`?kind=movie|tv`) and Added
/ Rating (`?sort=rating`). The list lives in the same SQLite database; each
title's name, year and poster are a snapshot of its cached TMDB entry, taken
when it's added. If the list can't be read, the page says so rather than
showing it empty.

## Favorites

Heart a movie or TV show on its detail page, in search results, beside a
filmography credit, or in either saved list. Favorites are independent of To watch and persist in
SQLite. Open Favorites from the header; filter All / Movies / TV and sort by
Added (newest first) or Rating, just like To watch. Unhearting removes a title
from Favorites only, keeping watched history. Favoriting a TV show also marks its
currently aired seasons (including specials) watched; undated seasons with listed
episodes count as current. Empty and future-dated seasons stay unwatched, and
seasons added later are not automatically watched. These season marks and the
favorite save together, without setting the whole-show watched flag. The show
page refreshes its season badges immediately after favoriting. If favorite state
cannot be read, hearts are disabled
rather than showing an incorrect unsaved state; list and save failures are
reported explicitly.

## Routes

| HTML | JSON |
|------|------|
| `/?q=…` (full page, or results fragment for HTMX) | `/api/search?q=…` |
| `/movie/{id}` | `/api/movie/{id}` |
| `/tv/{id}` | `/api/tv/{id}` |
| `/movie/{id}/credits`, `/tv/{id}/credits` | `/api/movie/{id}/credits`, `/api/tv/{id}/credits` |
| `/tv/{id}/season/{n}` | `/api/tv/{id}/season/{n}` |
| `/tv/{id}/season/{n}/episode/{e}` | `/api/tv/{id}/season/{n}/episode/{e}` |
| `/person/{id}` (`?sort=rating`) | `/api/person/{id}` (`?sort=rating`) |
| `/watchlist` (`?kind=movie\|tv`, `?sort=rating`, `?services=mine`) | `/api/watchlist` (same parameters) |
| `/favorites` (`?kind=movie\|tv`, `?sort=rating`) | `/api/favorites` (same parameters) |
| `/services` | `/api/services` |
| `/playback` | `/api/playback` |
| `POST /playback/decide` (form: `app`, `show`, `tv_id` or `other_id`, `action=use\|ignore`) | |
| `POST /services` (form: one `service={provider id}` per chosen service) | |
| `POST /movie/{id}/favorite`, `POST /tv/{id}/favorite` (form: `listed=true\|false`) | |
| `POST /movie/{id}/watchlist`, `POST /tv/{id}/watchlist` (form: `listed=true\|false`) | |
| `POST /tv/{id}/watched`, `POST /tv/{id}/season/{n}/watched` (form: `watched=true\|false`) | |

Both sides use `Catalog` (`src/catalog.rs`) and the types in `src/model.rs`.
Detail responses include a `ratings` object, key crew (directors and writers
for movies; creators and most frequent directors for TV), the top-billed cast,
cast/crew totals and, for TV, the season list. Episode responses carry that
episode's guest stars, the series regulars credited on it, its directors and
writers, and its crew grouped by department. Credits responses carry the full
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
