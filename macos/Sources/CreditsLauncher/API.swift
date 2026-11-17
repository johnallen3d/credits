import Foundation

// The subset of the credits JSON API (openapi.json) the launcher shows.

enum Kind: String, Decodable {
    case movie, tv, person
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
    let posterUrl: String?
    let profileUrl: String?
    let knownForDepartment: String?
    let knownFor: [KnownFor]?

    var displayName: String { title ?? name ?? "Untitled" }
    var imageUrl: String? { posterUrl ?? profileUrl }

    var kindLabel: String {
        switch kind {
        case .movie: return "Movie"
        case .tv: return "TV"
        case .person: return knownForDepartment ?? "Person"
        }
    }

    var details: String {
        switch kind {
        case .movie, .tv: return year ?? ""
        case .person: return (knownFor ?? []).map(\.title).joined(separator: ", ")
        }
    }

    static func == (a: SearchHit, b: SearchHit) -> Bool { a.kind == b.kind && a.id == b.id }
    func hash(into h: inout Hasher) { h.combine(kind); h.combine(id) }
}

struct KnownFor: Decodable {
    let title: String
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
}

private struct ErrorBody: Decodable { let error: String }

struct APIError: LocalizedError {
    let message: String
    var errorDescription: String? { message }
}

struct CreditsAPI {
    let base: URL

    /// The server in the terminal client's config file, found the same way
    /// (`$CREDITS_CONFIG`, else `$XDG_CONFIG_HOME/credits/config.toml`, else
    /// `~/.config/credits/config.toml`), else the hosted one.
    static func fromConfig() -> CreditsAPI {
        let fallback = URL(string: "https://credits.jallen7usa.com")!
        let env = ProcessInfo.processInfo.environment.filter { !$0.value.isEmpty }
        let path = env["CREDITS_CONFIG"].map { URL(fileURLWithPath: $0) }
            ?? env["XDG_CONFIG_HOME"].map { URL(fileURLWithPath: $0).appendingPathComponent("credits/config.toml") }
            ?? FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".config/credits/config.toml")
        guard let text = try? String(contentsOf: path, encoding: .utf8) else {
            return CreditsAPI(base: fallback)
        }
        for line in text.split(separator: "\n") {
            let parts = line.split(separator: "=", maxSplits: 1)
            guard parts.count == 2, parts[0].trimmingCharacters(in: .whitespaces) == "url" else { continue }
            let value = parts[1].split(separator: "#")[0]
                .trimmingCharacters(in: .whitespaces)
                .trimmingCharacters(in: CharacterSet(charactersIn: "\""))
            if let url = URL(string: value) { return CreditsAPI(base: url) }
        }
        return CreditsAPI(base: fallback)
    }

    func webURL(_ kind: Kind, _ id: Int) -> URL {
        base.appendingPathComponent("\(kind.rawValue)/\(id)")
    }

    func search(_ query: String) async throws -> [SearchHit] {
        var url = URLComponents(url: base.appendingPathComponent("api/search"), resolvingAgainstBaseURL: false)!
        url.queryItems = [URLQueryItem(name: "q", value: query)]
        let results: SearchResults = try await get(url.url!)
        return results.results
    }

    func detail(_ kind: Kind, _ id: Int) async throws -> Detail {
        let url = base.appendingPathComponent("api/\(kind.rawValue)/\(id)")
        switch kind {
        case .person: return .person(try await get(url))
        case .movie, .tv: return .title(try await get(url))
        }
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
