import AppIntents
import AppKit
import CoreSpotlight

// Siri, Shortcuts and Spotlight. The intents run in the app and call the same
// JSON API as the panel. Their metadata is extracted at build time into
// macos/Metadata.appintents (see `mise run mac:intents`), which the build
// copies into the app; regenerate it after changing anything here.

/// The running app, for intents that open the panel or act on its page.
@MainActor
enum Launcher {
    static weak var app: AppDelegate?
    static var model: LauncherModel? { app?.model }
}

extension APIError: CustomLocalizedStringResourceConvertible {
    /// Siri and Shortcuts show this, so a server that can't be reached is said
    /// rather than failing silently.
    var localizedStringResource: LocalizedStringResource { "\(message)" }
}

/// A movie or TV show, as Siri, Shortcuts and Spotlight see it.
struct TitleEntity: AppEntity, IndexedEntity {
    static let typeDisplayRepresentation = TypeDisplayRepresentation(name: "Title", numericFormat: "\(placeholder: .int) titles")
    static let defaultQuery = TitleQuery()

    /// "movie/438631" or "tv/1396", as in the site's paths.
    let id: String
    @Property(title: "Name") var name: String
    @Property(title: "Year") var year: String?
    @Property(title: "Kind") var kindName: String
    @Property(title: "Overview") var overview: String?
    /// The lists it's on, when it came from one: To watch, Favorites, Watched.
    @Property(title: "Lists") var lists: [String]
    var posterUrl: String?

    init(kind: Kind, id: Int, name: String, year: String?, overview: String? = nil,
         posterUrl: String? = nil, lists: [String] = []) {
        self.id = "\(kind.rawValue)/\(id)"
        self.posterUrl = posterUrl
        self.name = name
        self.year = year
        self.kindName = kind == .tv ? "TV show" : "Movie"
        self.overview = overview
        self.lists = lists
    }

    var kind: Kind { id.hasPrefix("tv/") ? .tv : .movie }
    var tmdbId: Int { Int(id.split(separator: "/").last ?? "") ?? 0 }
    var hit: SearchHit { SearchHit(kind: kind, id: tmdbId, title: name, year: year, posterUrl: posterUrl) }

    var displayRepresentation: DisplayRepresentation {
        let subtitle = [kindName, year].compactMap { $0 }.joined(separator: " · ")
        return DisplayRepresentation(title: "\(name)", subtitle: "\(subtitle)",
                                     image: posterUrl.flatMap(URL.init(string:)).map { .init(url: $0) })
    }

    var attributeSet: CSSearchableItemAttributeSet {
        let set = CSSearchableItemAttributeSet()
        set.displayName = name
        set.contentDescription = [[kindName, year].compactMap { $0 }.joined(separator: " · "),
                                  lists.isEmpty ? nil : "On " + lists.joined(separator: ", "),
                                  overview].compactMap { $0 }.joined(separator: "\n")
        set.keywords = lists + [kindName]
        return set
    }

    init(_ hit: SearchHit) {
        self.init(kind: hit.kind, id: hit.id, name: hit.displayName, year: hit.year,
                  overview: hit.overview, posterUrl: hit.posterUrl)
    }

    init(_ t: TitleDetail) {
        self.init(kind: t.kind, id: t.id, name: t.title, year: t.year, overview: t.overview, posterUrl: t.posterUrl)
    }
}

/// Finds titles by TMDB search; the catalog lives on the server and changes,
/// so it isn't indexed ahead of time.
struct TitleQuery: EntityStringQuery {
    func entities(for identifiers: [TitleEntity.ID]) async throws -> [TitleEntity] {
        let api = CreditsAPI.fromConfig()
        return try await withThrowingTaskGroup(of: TitleEntity?.self) { group in
            for id in identifiers {
                let parts = id.split(separator: "/")
                guard parts.count == 2, let kind = Kind(rawValue: String(parts[0])),
                      kind == .movie || kind == .tv, let n = Int(parts[1]) else { continue }
                group.addTask {
                    guard case .title(let t) = try await api.detail(SearchHit(kind: kind, id: n, title: nil))
                    else { return nil }
                    return TitleEntity(t)
                }
            }
            return try await group.reduce(into: []) { if let e = $1 { $0.append(e) } }
        }
    }

    func entities(matching string: String) async throws -> [TitleEntity] {
        try await CreditsAPI.fromConfig().search(string).filter(\.isTitle).map(TitleEntity.init)
    }

    /// To watch, so the picker opens on what you're likely to choose.
    func suggestedEntities() async throws -> [TitleEntity] {
        try await CreditsAPI.fromConfig().watchlist(sort: .added).titles.map {
            TitleEntity(kind: $0.kind, id: $0.id, name: $0.title, year: $0.year, posterUrl: $0.posterUrl,
                        lists: [ListAction.toWatch.name])
        }
    }
}

/// The title open in the panel, if it's showing one.
@MainActor
private func openTitle() -> TitleEntity? {
    guard let model = Launcher.model, Launcher.app?.panelVisible == true,
          let hit = model.detailHit, hit.isTitle else { return nil }
    if case .title(let t) = model.detail, t.id == hit.id { return TitleEntity(t) }
    return TitleEntity(hit)
}

/// Puts a title on To watch or takes it off, then tells the panel and Spotlight.
private func setToWatch(_ title: TitleEntity?, _ on: Bool,
                        ask: () -> Error) async throws -> some IntentResult & ProvidesDialog {
    var target = title
    if target == nil { target = await openTitle() }
    guard let target else { throw ask() }
    let marks = try await CreditsAPI.fromConfig().setList(.toWatch, target.kind, target.tmdbId, on)
    await MainActor.run { Launcher.model?.listChangedElsewhere(target.hit, marks) }
    await SpotlightIndex.refresh()
    let done = ListAction.toWatch.done(on)
    return .result(dialog: "\(done): \(target.name)")
}

struct AddToWatchIntent: AppIntent {
    static let title: LocalizedStringResource = "Add to To Watch"
    static let description = IntentDescription("Puts a movie or TV show on your Credits To watch list. With no title, adds the one open in the launcher.")

    @Parameter(title: "Title") var title: TitleEntity?

    static var parameterSummary: some ParameterSummary { Summary("Add \(\.$title) to To watch") }

    func perform() async throws -> some IntentResult & ProvidesDialog {
        try await setToWatch(title, true) { $title.needsValueError("Which movie or show?") }
    }
}

struct RemoveFromToWatchIntent: AppIntent {
    static let title: LocalizedStringResource = "Remove from To Watch"
    static let description = IntentDescription("Takes a movie or TV show off your Credits To watch list. With no title, removes the one open in the launcher.")

    @Parameter(title: "Title") var title: TitleEntity?

    static var parameterSummary: some ParameterSummary { Summary("Remove \(\.$title) from To watch") }

    func perform() async throws -> some IntentResult & ProvidesDialog {
        try await setToWatch(title, false) { $title.needsValueError("Which movie or show?") }
    }
}

struct ToWatchListIntent: AppIntent {
    static let title: LocalizedStringResource = "Get To Watch"
    static let description = IntentDescription("Returns the movies and TV shows on your Credits To watch list, most recently added first.")

    func perform() async throws -> some IntentResult & ReturnsValue<[TitleEntity]> & ProvidesDialog {
        let titles = try await TitleQuery().suggestedEntities()
        guard !titles.isEmpty else { return .result(value: [], dialog: "Your To watch list is empty.") }
        let names = titles.prefix(5).map(\.name).joined(separator: ", ")
        let more = titles.count > 5 ? ", and \(titles.count - 5) more" : ""
        return .result(value: titles, dialog: "\(titles.count) on To watch: \(names)\(more).")
    }
}

struct SearchTitlesIntent: AppIntent {
    static let title: LocalizedStringResource = "Search Titles"
    static let description = IntentDescription("Searches TMDB for movies and TV shows by name.")

    @Parameter(title: "Name", requestValueDialog: "What should I search for?") var query: String

    static var parameterSummary: some ParameterSummary { Summary("Search Credits for \(\.$query)") }

    func perform() async throws -> some IntentResult & ReturnsValue<[TitleEntity]> & ProvidesDialog {
        let titles = try await TitleQuery().entities(matching: query)
        guard let first = titles.first else { return .result(value: [], dialog: "No titles match \(query).") }
        func named(_ t: TitleEntity) -> String { [t.name, t.year].compactMap { $0 }.joined(separator: ", ") }
        let rest = titles.dropFirst().prefix(2).map(named)
        let also = rest.isEmpty ? "" : " Also: " + rest.joined(separator: "; ") + "."
        let top = named(first)
        return .result(value: titles, dialog: "Top match: \(top), \(first.kindName.lowercased()).\(also)")
    }
}

/// Opens a title's page in the launcher, as choosing it in Spotlight does.
struct OpenTitleIntent: OpenIntent {
    static let title: LocalizedStringResource = "Open Title"
    static let description = IntentDescription("Opens a movie's or TV show's page in the Credits launcher.")

    @Parameter(title: "Title") var target: TitleEntity

    @MainActor
    func perform() async throws -> some IntentResult {
        Launcher.app?.open(target.hit)
        return .result()
    }
}

struct CreditsShortcuts: AppShortcutsProvider {
    static var appShortcuts: [AppShortcut] {
        AppShortcut(intent: AddToWatchIntent(), phrases: [
            "Add \(\.$title) to my \(.applicationName) watchlist",
            "Add \(\.$title) to To watch in \(.applicationName)",
            "Add this to my \(.applicationName) watchlist",
            "Add to To watch in \(.applicationName)",
        ], shortTitle: "Add to To Watch", systemImageName: "bookmark")
        AppShortcut(intent: RemoveFromToWatchIntent(), phrases: [
            "Remove \(\.$title) from my \(.applicationName) watchlist",
            "Remove this from my \(.applicationName) watchlist",
        ], shortTitle: "Remove from To Watch", systemImageName: "bookmark.slash")
        AppShortcut(intent: ToWatchListIntent(), phrases: [
            "What's on my \(.applicationName) watchlist",
            "What's on To watch in \(.applicationName)",
            "Show my \(.applicationName) watchlist",
        ], shortTitle: "To Watch", systemImageName: "list.bullet")
        AppShortcut(intent: SearchTitlesIntent(), phrases: [
            "Search \(.applicationName)",
            "Search titles in \(.applicationName)",
        ], shortTitle: "Search Titles", systemImageName: "magnifyingglass")
    }
}

/// Keeps To watch, Favorites and Watched in Spotlight's index, where Siri can
/// answer questions about them. A failed list keeps its last indexed entries.
enum SpotlightIndex {
    private static let state = State()

    private actor State {
        var running: Task<Void, Never>?
        func refresh() async {
            if let running { return await running.value }
            let task = Task { await SpotlightIndex.rebuild() }
            running = task
            await task.value
            running = nil
        }
    }

    static func refresh() async { await state.refresh() }

    private static func rebuild() async {
        let api = CreditsAPI.fromConfig()
        async let toWatch = try? api.watchlist(sort: .added)
        async let favorites = try? api.favorites()
        async let watched = try? api.watched(sort: .recent)
        let (t, f, w) = await (toWatch, favorites, watched)
        guard t != nil || f != nil || w != nil else {
            NSLog("CreditsLauncher: Spotlight index not updated; the server couldn't be reached")
            return
        }
        var byId: [String: TitleEntity] = [:]
        var order: [String] = []
        func add(_ kind: Kind, _ id: Int, _ name: String?, _ year: String?, _ poster: String?, _ list: ListAction) {
            guard let name else { return }
            let entity = TitleEntity(kind: kind, id: id, name: name, year: year, posterUrl: poster)
            if byId[entity.id] == nil { order.append(entity.id); byId[entity.id] = entity }
            byId[entity.id]!.lists.append(list.name)
        }
        for e in t?.titles ?? [] { add(e.kind, e.id, e.title, e.year, e.posterUrl, .toWatch) }
        for e in f?.titles ?? [] { add(e.kind, e.id, e.title, e.year, e.posterUrl, .favorite) }
        for e in w?.titles ?? [] { add(e.kind, e.id, e.title, e.year, e.posterUrl, .watched) }
        let index = CSSearchableIndex.default()
        do {
            // Only start over when every list loaded, so one failure doesn't
            // drop the others' titles.
            if t != nil && f != nil && w != nil { try await index.deleteAppEntities(ofType: TitleEntity.self) }
            try await index.indexAppEntities(order.compactMap { byId[$0] })
        } catch {
            NSLog("CreditsLauncher: couldn't update the Spotlight index: \(error)")
        }
    }
}

/// Tells the system which title the panel shows, so "add this to my
/// watchlist" knows what "this" is.
@MainActor
final class OnScreenTitle {
    private var activity: NSUserActivity?

    func update(_ hit: SearchHit?, visible: Bool) {
        guard visible, let hit, hit.isTitle else {
            activity?.resignCurrent()
            activity = nil
            return
        }
        let entity = TitleEntity(hit)
        if activity?.appEntityIdentifier == EntityIdentifier(for: entity) { return }
        activity?.resignCurrent()
        let a = NSUserActivity(activityType: "com.jallen7usa.credits.launcher.title")
        a.title = entity.name
        a.appEntityIdentifier = EntityIdentifier(for: entity)
        a.becomeCurrent()
        activity = a
    }
}
