import Foundation

// The subset of the credits JSON API (openapi.json) the launcher shows.

enum Kind: String, Decodable {
    case movie, tv, person
    /// Pages inside a show, which the API never sends as a kind: a season, and
    /// an episode (opened on the web only).
    case season, episode
}

struct SearchResults: Decodable {
    let results: [SearchHit]
}

struct SearchHit: Decodable, Identifiable, Hashable {
    let kind: Kind
    let id: Int
    let title: String?
    let name: String?
    let year: String?
    let overview: String?
    let inTheaters: Bool?
    let posterUrl: String?
    let profileUrl: String?
    let knownForDepartment: String?
    let knownFor: [KnownFor]?
    /// A list entry's own line under its name, in place of `summary`.
    var info: String? = nil
    /// For a season or episode, its season number; `id` is the show's.
    var season: Int? = nil
    /// For an episode, its number within the season.
    var episode: Int? = nil

    var displayName: String { title ?? name ?? "Untitled" }
    var imageUrl: String? { posterUrl ?? profileUrl }
    /// A movie or show, which can go on lists.
    var isTitle: Bool { kind == .movie || kind == .tv }

    /// The type badge, as on the web's search results.
    var kindLabel: String {
        switch kind {
        case .movie: return "Movie"
        case .tv: return "TV show"
        case .person: return "Person"
        case .season: return "TV season"
        case .episode: return "Episode"
        }
    }

    /// A title's overview, or a person's department and best-known titles, as on the web.
    var summary: String? {
        if let info { return info.isEmpty ? nil : info }
        switch kind {
        case .movie, .tv, .season, .episode: return overview.flatMap { $0.isEmpty ? nil : $0 }
        case .person:
            let titles = (knownFor ?? []).map { k in k.year.map { "\(k.title) (\($0))" } ?? k.title }
                .joined(separator: ", ")
            switch (knownForDepartment, titles.isEmpty) {
            case (let d?, false): return "\(d) · known for \(titles)"
            case (let d?, true): return d
            case (nil, false): return "Known for \(titles)"
            case (nil, true): return nil
            }
        }
    }

    /// A credit on a detail page, to open as a page of its own.
    init(kind: Kind, id: Int, title: String? = nil, name: String? = nil, year: String? = nil,
         posterUrl: String? = nil, info: String? = nil, season: Int? = nil, episode: Int? = nil) {
        self.kind = kind
        self.id = id
        self.title = title
        self.name = name
        self.year = year
        self.posterUrl = posterUrl
        self.info = info
        self.season = season
        self.episode = episode
        overview = nil
        inTheaters = nil
        profileUrl = nil
        knownForDepartment = nil
        knownFor = nil
    }

    static func == (a: SearchHit, b: SearchHit) -> Bool {
        a.kind == b.kind && a.id == b.id && a.season == b.season && a.episode == b.episode
    }
    func hash(into h: inout Hasher) { h.combine(kind); h.combine(id); h.combine(season); h.combine(episode) }
}

struct KnownFor: Decodable {
    let title: String
    let year: String?
}

struct Metadata: Decodable {
    let state: String
    let fetchedAt: String
}

struct TitleDetail: Decodable {
    let kind: Kind
    let id: Int
    let title: String
    let tagline: String?
    let overview: String?
    let year: String?
    let status: String?
    let genres: [String]
    let runtimeMinutes: Int?
    let seasons: Int?
    let episodes: Int?
    let releases: Releases?
    let posterUrl: String?
    let ratings: Ratings?
    let whereToWatch: WhereToWatch?
    let keyCrew: [KeyCrew]
    let cast: [Credit]
    let metadata: Metadata
    let marks: TitleMarks?
    /// On To watch only: tags naming who or what recommended it.
    let sources: [String]?
    /// TV only: regular seasons first, specials (season 0) last.
    let seasonList: [SeasonSummary]?
    /// TV only.
    let watched: WatchedMarks?
}

struct SeasonSummary: Decodable, Hashable {
    let number: Int
    let name: String
    let year: String?
    let episodeCount: Int?
}

/// A show's watched marks, for hiding spoilers as the web does. Marks cover
/// what's under them: a watched show its seasons, a watched season its episodes.
struct WatchedMarks: Decodable {
    let state: String
    let show: Bool
    let seasons: [Int]
    let episodes: [EpisodeMark]

    func season(_ n: Int) -> Bool { show || seasons.contains(n) }

    func episode(_ season: Int, _ n: Int) -> Bool {
        self.season(season) || episodes.contains { $0.season == season && $0.number == n }
    }

    /// Whether the episode's own mark came from what played on the Apple TV.
    func fromAppleTV(_ season: Int, _ n: Int) -> Bool {
        episodes.contains { $0.season == season && $0.number == n && $0.appleTv }
    }

    /// The share of a season in progress watched, as the web shows it: nil when
    /// none or all of it is.
    func percent(_ season: Int, of count: Int?) -> Int? {
        guard let total = count, total > 0, !self.season(season) else { return nil }
        let seen = episodes.filter { $0.season == season && $0.number <= total }.count
        return seen > 0 && seen < total ? seen * 100 / total : nil
    }
}

struct EpisodeMark: Decodable {
    let season: Int
    let number: Int
    let appleTv: Bool
}

struct SeasonDetail: Decodable {
    let showId: Int
    let showTitle: String
    let number: Int
    let name: String
    let overview: String?
    let airDate: String?
    let posterUrl: String?
    let episodes: [Episode]
    let episodeRatings: EpisodeRatings
    let previous: SeasonSummary?
    let next: SeasonSummary?
    let metadata: Metadata
    let watched: WatchedMarks?
}

struct Episode: Decodable, Hashable {
    let number: Int
    let title: String
    let airDate: String?
    let overview: String?
    let runtimeMinutes: Int?
    let stillUrl: String?
    let rating: Rating?
}

struct EpisodeRatings: Decodable {
    let state: String
    let unavailableReason: String?
    let fetchedAt: String?

    /// Why scores are missing or old, worded as on the web; nil when they're fresh.
    var note: String? {
        switch state {
        case "unavailable":
            switch unavailableReason {
            case "not_configured": return "Ratings aren't configured on this server."
            case "no_imdb_id": return "No IMDb entry is linked to this show."
            default: return "Episode ratings couldn't be loaded right now."
            }
        case "stale":
            return "Couldn't refresh episode ratings" + (fetchedAt.map { "; showing scores from \(shortDate($0))" } ?? "") + "."
        default: return nil
        }
    }
}

/// Whether a title is on To watch and Favorites and marked watched. Each is
/// nil when the server couldn't read it.
struct TitleMarks: Decodable, Equatable {
    var toWatch: Bool?
    var favorite: Bool?
    var watched: Bool?

    subscript(list: ListAction) -> Bool? {
        switch list {
        case .toWatch: return toWatch
        case .favorite: return favorite
        case .watched: return watched
        }
    }
}

private struct TitleMarksUpdate: Decodable { let marks: TitleMarks }

/// The lists the launcher can change, and their ⌘ shortcuts.
enum ListAction: CaseIterable {
    case toWatch, favorite, watched

    var name: String {
        switch self {
        case .toWatch: return "To watch"
        case .favorite: return "Favorites"
        case .watched: return "Watched"
        }
    }

    var key: String {
        switch self {
        case .toWatch: return "t"
        case .favorite: return "f"
        case .watched: return "w"
        }
    }

    var icon: String {
        switch self {
        case .toWatch: return "bookmark"
        case .favorite: return "heart"
        case .watched: return "eye"
        }
    }

    fileprivate var path: String {
        switch self {
        case .toWatch: return "watchlist"
        case .favorite: return "favorite"
        case .watched: return "watched"
        }
    }

    fileprivate var bodyKey: String { self == .watched ? "watched" : "listed" }

    /// What happened, for the confirmation line.
    func done(_ on: Bool) -> String {
        switch self {
        case .toWatch, .favorite: return on ? "Added to \(name)" : "Removed from \(name)"
        case .watched: return on ? "Marked watched" : "Unmarked watched"
        }
    }
}

struct Releases: Decodable {
    let certification: String?
}

struct Ratings: Decodable {
    let state: String
    let preferred: Rating?
    let others: [Rating]
}

struct Rating: Decodable, Hashable {
    let source: String
    let score: Double
    let scale: Int

    var label: String {
        switch source {
        case "imdb": return "IMDb \(String(format: "%.1f", score))"
        case "rotten_tomatoes": return "RT \(Int(score.rounded()))%"
        default: return "\(source) \(score)/\(scale)"
        }
    }
}

struct WhereToWatch: Decodable {
    let state: String
    let offers: [OfferGroup]
}

struct OfferGroup: Decodable, Hashable {
    let kind: String
    let providers: [Provider]
}

struct Provider: Decodable, Hashable {
    let name: String
    let mine: Bool
}

struct KeyCrew: Decodable, Hashable {
    let role: String
    let people: [Credit]
}

struct Credit: Decodable, Hashable {
    let personId: Int
    let name: String
    let role: String?
}

struct PersonDetail: Decodable {
    let id: Int
    let name: String
    let knownFor: String?
    let biography: String?
    let birthday: String?
    let deathday: String?
    let placeOfBirth: String?
    let profileUrl: String?
    let knownForTitles: [PersonCredit]
    let metadata: Metadata
}

struct PersonCredit: Decodable, Hashable {
    let kind: Kind
    let id: Int
    let title: String
    let year: String?
    let role: String?
}

enum Detail {
    case title(TitleDetail)
    case person(PersonDetail)
    case season(SeasonDetail)

    /// How many cast members a title's page lists.
    static let castShown = 8

    /// The rows a page lists, in order: a title's key crew, seasons and cast;
    /// a person's titles; or a season's neighbours and episodes. Each opens a
    /// page of its own, except episodes, which open on the web.
    var rows: [SearchHit] {
        switch self {
        case .title(let t):
            let person = { (c: Credit) in SearchHit(kind: .person, id: c.personId, name: c.name) }
            return t.keyCrew.flatMap(\.people).map(person)
                + (t.seasonList ?? []).map { SearchHit(kind: .season, id: t.id, title: $0.name, season: $0.number) }
                + t.cast.prefix(Self.castShown).map(person)
        case .person(let p):
            return p.knownForTitles.map { SearchHit(kind: $0.kind, id: $0.id, title: $0.title, year: $0.year) }
        case .season(let s):
            return [s.previous, s.next].compactMap { $0 }
                .map { SearchHit(kind: .season, id: s.showId, title: $0.name, season: $0.number) }
                + s.episodes.map {
                    SearchHit(kind: .episode, id: s.showId, title: $0.title, season: s.number, episode: $0.number)
                }
        }
    }
}

// The lists the empty panel shows.

struct RecentViews: Decodable {
    let recent: [RecentView]
}

struct RecentView: Decodable {
    let kind: Kind
    let id: Int
    let name: String
    let year: String?
    let imageUrl: String?
}

struct Watchlist: Decodable {
    let titles: [ListEntry]
    let services: ListServices?
}

struct ListEntry: Decodable {
    let kind: Kind
    let id: Int
    let title: String
    let year: String?
    let posterUrl: String?
    let availability: Availability?
    let marks: TitleMarks?
    /// On To watch only: tags naming who or what recommended it.
    let sources: [String]?
}

struct Availability: Decodable {
    let services: [Service]
}

struct Service: Decodable {
    let name: String
}

struct ListServices: Decodable {
    let state: String
    let unknown: Int
    let staleSince: String?
}

struct NowPlaying: Decodable {
    let titles: [ReleaseEntry]
    let ratings: SortRatings
    let unchecked: Int
}

struct ReleaseEntry: Decodable {
    let id: Int
    let title: String
    let releaseDate: String?
    let posterUrl: String?
    let rating: Rating?
}

struct SortRatings: Decodable {
    let staleSince: String?
    let unavailableReason: String?
}

private struct ErrorBody: Decodable { let error: String }

struct APIError: LocalizedError {
    let message: String
    var errorDescription: String? { message }
}

struct CreditsAPI {
    let base: URL

    /// The server in the terminal client's config file, else the hosted one.
    static func fromConfig() -> CreditsAPI {
        let fallback = URL(string: "https://credits.jallen7usa.com")!
        return CreditsAPI(base: CreditsConfig.value("url").flatMap(URL.init(string:)) ?? fallback)
    }

    /// A page's path under the site, which the API mirrors under `api/`.
    private func path(_ hit: SearchHit) -> String {
        switch hit.kind {
        case .movie, .tv, .person: return "\(hit.kind.rawValue)/\(hit.id)"
        case .season: return "tv/\(hit.id)/season/\(hit.season ?? 0)"
        case .episode: return "tv/\(hit.id)/season/\(hit.season ?? 0)/episode/\(hit.episode ?? 0)"
        }
    }

    func webURL(_ hit: SearchHit) -> URL {
        base.appendingPathComponent(path(hit))
    }

    func search(_ query: String) async throws -> [SearchHit] {
        var url = URLComponents(url: base.appendingPathComponent("api/search"), resolvingAgainstBaseURL: false)!
        url.queryItems = [URLQueryItem(name: "q", value: query)]
        let results: SearchResults = try await get(url.url!)
        return results.results
    }

    func detail(_ hit: SearchHit) async throws -> Detail {
        let url = base.appendingPathComponent("api/" + path(hit))
        switch hit.kind {
        case .person: return .person(try await get(url))
        case .movie, .tv: return .title(try await get(url))
        case .season: return .season(try await get(url))
        case .episode: throw APIError(message: "Episodes open on the web")
        }
    }

    func recent() async throws -> RecentViews {
        try await get(base.appendingPathComponent("api/recent"))
    }

    func watchlist() async throws -> Watchlist {
        try await get(base.appendingPathComponent("api/watchlist"))
    }

    func nowPlaying() async throws -> NowPlaying {
        try await get(base.appendingPathComponent("api/now-playing"))
    }

    /// Puts a movie or show on a list or takes it off; returns its marks after.
    func setList(_ list: ListAction, _ kind: Kind, _ id: Int, _ on: Bool) async throws -> TitleMarks {
        var request = URLRequest(url: base.appendingPathComponent("api/\(kind.rawValue)/\(id)/\(list.path)"))
        request.httpMethod = "PUT"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONSerialization.data(withJSONObject: [list.bodyKey: on])
        let update: TitleMarksUpdate = try await send(request)
        return update.marks
    }

    private func get<T: Decodable>(_ url: URL) async throws -> T {
        try await send(URLRequest(url: url))
    }

    private func send<T: Decodable>(_ request: URLRequest) async throws -> T {
        let data: Data
        let response: URLResponse
        do {
            (data, response) = try await URLSession.shared.data(for: request)
        } catch let error as URLError where error.code == .cancelled {
            throw CancellationError()
        } catch {
            throw APIError(message: "Can't reach \(base.host() ?? base.absoluteString): \(error.localizedDescription)")
        }
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        if let http = response as? HTTPURLResponse, http.statusCode != 200 {
            let message = (try? decoder.decode(ErrorBody.self, from: data))?.error ?? "HTTP \(http.statusCode)"
            throw APIError(message: message)
        }
        do {
            return try decoder.decode(T.self, from: data)
        } catch {
            throw APIError(message: "Unexpected response from the server")
        }
    }
}
