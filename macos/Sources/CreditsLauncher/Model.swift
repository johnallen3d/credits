import AppKit
import Observation

@MainActor
@Observable
final class LauncherModel {
    let api = CreditsAPI.fromConfig()

    var query = "" {
        didSet { if query != oldValue { scheduleSearch() } }
    }
    var results: [SearchHit] = []
    var selection: SearchHit?
    var searching = false
    var searchError: String?

    /// With no query the panel shows a list instead: one per tab.
    var tab: HomeTab = .recent
    var home: [HomeTab: HomeList] = [:]
    /// Why a tab's last load failed, shown over its saved list if it has one.
    var homeError: [HomeTab: String] = [:]
    var homeLoading: Set<HomeTab> = []

    /// To watch as the server last sent it, in `toWatchSort` order; the tab
    /// shows the titles `toWatchFilter` keeps.
    var toWatch: Watchlist?
    var toWatchSort: WatchlistSort = .added {
        didSet { if toWatchSort != oldValue { reloadHome(.toWatch) } }
    }
    var toWatchFilter = ToWatchFilter() {
        didSet { rebuildToWatch() }
    }

    var showingHome: Bool { query.trimmingCharacters(in: .whitespaces).isEmpty }
    /// What ↑↓ and ⏎ act on: the search results, or the tab's list.
    var rows: [SearchHit] { showingHome ? home[tab]?.hits ?? [] : results }

    /// The page in detail's recommended-by tag being typed, and whether its
    /// field has the keys (so ⏎ and esc go to it).
    var tagDraft = ""
    var editingTags = false
    /// Bumped by ⌘R to put the keys in the tag field, and by esc there to give
    /// them back to the search field.
    var focusTags = 0
    var focusSearch = 0
    /// Tags used on To watch so far, offered on a title's page.
    var knownTags: [String] = []

    /// The page in detail: a result, or a credit opened from another page.
    var detailHit: SearchHit?
    var detail: Detail?
    var detailError: String?
    /// The selected credit on the page in detail, an index into `detailRows`.
    var detailRow: Int?
    /// On a person's page: the filmography's Show filter, Movies or TV (nil for all).
    var personKind: Kind? {
        didSet { if personKind != oldValue { detailRow = nil } }
    }
    /// On a person's page: whether another sort is loading, and why the last one failed.
    var personSorting = false
    var personSortError: String?
    /// On a person's page: the lists whose marks couldn't be read for its credits.
    var personListsUnread: [ListAction] = []
    /// On an episode's page: whether its spoilers show though it isn't watched.
    var peeking = false
    /// The pages behind the one in detail, oldest first.
    var back: [Page] = []
    /// Bumped on every page change, so the view starts each page at its top.
    var pageID = 0

    struct Page {
        let hit: SearchHit
        var detail: Detail?
        let error: String?
        let row: Int?
        var kind: Kind? = nil
        var listsUnread: [ListAction] = []
    }

    /// The rows of the page in detail, as `Detail.rows` numbers them.
    var detailRows: [SearchHit] { detail?.rows(kind: personKind) ?? [] }

    /// Summaries for rows whose list didn't carry one (Recently viewed), from each
    /// page's detail, fetched as the row first shows.
    var summaries: [SearchHit: String] = [:]
    private var summaryAsked: Set<SearchHit> = []

    /// List state per title, from its detail or the last change.
    var marks: [SearchHit: TitleMarks] = [:]
    /// Titles with a list change in flight, and episodes with a watched change.
    var changing: Set<SearchHit> = []
    /// A one-line result of the last list action.
    var flash: Flash?

    struct Flash: Equatable {
        let text: String
        let failed: Bool
        let id = UUID()
    }

    /// An image open full size over the panel.
    var viewing: FullImage?

    struct FullImage: Equatable {
        /// The copy already on the page, shown until the original loads.
        let shown: URL
        let full: URL
        let label: String
    }

    /// Bumped each time the panel opens, so the view refocuses the field.
    var shown = 0
    var close: () -> Void = {}

    private var searchTask: Task<Void, Never>?
    private var detailTask: Task<Void, Never>?
    private var sortTask: Task<Void, Never>?
    private var flashTask: Task<Void, Never>?
    private var homeTasks: [HomeTab: Task<Void, Never>] = [:]

    func panelOpened() {
        shown += 1
        viewing = nil
        loadHome(tab)
    }

    /// Shows the next or previous tab's list, refreshing it.
    func switchTab(_ delta: Int) {
        let all = HomeTab.allCases
        let index = all.firstIndex(of: tab)!
        selectTab(all[(index + delta + all.count) % all.count])
    }

    func selectTab(_ next: HomeTab) {
        tab = next
        selection = rows.first
        loadHome(next)
    }

    /// Drops a load in flight (its answer would be for an old sort) and starts again.
    private func reloadHome(_ tab: HomeTab) {
        homeTasks[tab]?.cancel()
        homeTasks[tab] = nil
        loadHome(tab)
    }

    /// Reloads a tab's list, keeping the saved one on screen until it's back.
    private func loadHome(_ tab: HomeTab) {
        guard homeTasks[tab] == nil else { return }
        homeLoading.insert(tab)
        homeTasks[tab] = Task {
            defer {
                if !Task.isCancelled { homeLoading.remove(tab); homeTasks[tab] = nil }
            }
            do {
                let list = try await fetch(tab)
                guard !Task.isCancelled else { return }
                home[tab] = list
                homeError[tab] = nil
            } catch is CancellationError {
                return
            } catch {
                guard !Task.isCancelled else { return }
                homeError[tab] = error.localizedDescription
            }
            // Keep the selection where it was if it's still listed.
            if showingHome, self.tab == tab, selection.map({ !rows.contains($0) }) ?? true {
                selection = rows.first
            }
        }
    }

    private func fetch(_ tab: HomeTab) async throws -> HomeList {
        switch tab {
        case .recent:
            let r = try await api.recent()
            return HomeList(hits: r.recent.map {
                SearchHit(kind: $0.kind, id: $0.id, title: $0.name, year: $0.year, posterUrl: $0.imageUrl)
            })
        case .toWatch:
            let w = try await api.watchlist(sort: toWatchSort)
            for t in w.titles {
                if let m = t.marks { marks[SearchHit(kind: t.kind, id: t.id)] = m }
            }
            toWatch = w
            // A tag no title has any more can't match, so stop filtering by it.
            if let tag = toWatchFilter.tag, !toWatchTags.contains(where: { $0.lowercased() == tag.lowercased() }) {
                toWatchFilter.tag = nil
            }
            return toWatchList(w)
        case .nowPlaying:
            let n = try await api.nowPlaying()
            var notes: [String] = []
            if let since = n.ratings.staleSince { notes.append("Ratings saved from \(shortDate(since)): OMDb isn't responding") }
            switch n.ratings.unavailableReason {
            case "provider_error": notes.append("Some ratings couldn't be loaded")
            case "not_configured": notes.append("Ratings aren't set up on the server")
            default: break
            }
            if n.unchecked > 0 {
                notes.append("\(n.unchecked) movie\(n.unchecked == 1 ? "" : "s") left out: release dates couldn't be loaded")
            }
            return HomeList(hits: n.titles.map { m in
                let opened = m.releaseDate.map { "In theaters since \(shortDate($0))" }
                let info = [opened, m.rating?.label].compactMap { $0 }.joined(separator: " · ")
                return SearchHit(kind: .movie, id: m.id, title: m.title, posterUrl: m.posterUrl, info: info)
            }, notes: notes)
        }
    }

    /// The services filter applies once services are chosen.
    var servicesChosen: Bool { toWatch?.services?.state == "chosen" }

    /// Every tag on To watch, in the order first seen.
    var toWatchTags: [String] {
        var seen = Set<String>()
        return (toWatch?.titles ?? []).flatMap { $0.sources ?? [] }
            .filter { seen.insert($0.lowercased()).inserted }
    }

    /// How many To watch titles a filter keeps, for the filter menus' counts.
    func toWatchCount(_ filter: ToWatchFilter) -> Int {
        let filter = servicesChosen ? filter : filter.with(\.onMyServices, false)
        return (toWatch?.titles ?? []).filter(filter.keeps).count
    }

    private func rebuildToWatch() {
        if let w = toWatch { home[.toWatch] = toWatchList(w) }
        if showingHome, tab == .toWatch, selection.map({ !rows.contains($0) }) ?? true {
            selection = rows.first
        }
    }

    /// To watch's titles that the filter keeps, with notes on what stood behind them.
    private func toWatchList(_ w: Watchlist) -> HomeList {
        let filter = servicesChosen ? toWatchFilter : toWatchFilter.with(\.onMyServices, false)
        var notes: [String] = []
        let leftOut: (Int) -> String = { $0 == 1 ? "; it's left out" : "; they're left out" }
        if let s = w.services {
            if s.state == "unreadable" { notes.append("Your streaming services couldn't be read, so titles can't be filtered by them") }
            if s.state == "not_chosen" { notes.append("Choose your streaming services on the web to filter by them") }
            if let since = s.staleSince { notes.append("Services saved from \(shortDate(since)): TMDB isn't responding") }
            if s.unknown > 0 && s.state == "chosen" {
                notes.append("Services couldn't be loaded for \(s.unknown) title\(s.unknown == 1 ? "" : "s")"
                             + (filter.onMyServices ? leftOut(s.unknown) : ""))
            }
        }
        if let n = w.releaseUnknown, n > 0 {
            notes.append("Release dates couldn't be loaded for \(n) title\(n == 1 ? "" : "s")"
                         + (filter.release != nil ? leftOut(n) : ""))
        }
        if let r = w.sortRatings {
            if let since = r.staleSince { notes.append("Ratings saved from \(shortDate(since)): OMDb isn't responding") }
            switch r.unavailableReason {
            case "not_configured": notes.append("Ratings aren't set up on the server, so titles stay in the order added")
            case .some: notes.append("Some ratings couldn't be loaded")
            case nil:
                if let n = r.unrated, n > 0 { notes.append("Titles without a score are listed last, most recently added first") }
            }
        }
        return HomeList(hits: w.titles.filter(filter.keeps).map { t in
            let services = (t.availability?.services ?? []).map(\.name)
            let tags = (t.sources ?? []).map { "#\($0)" }.joined(separator: " ")
            let info = ([t.rating?.label].compactMap { $0 } + services + [tags].filter { !$0.isEmpty })
                .joined(separator: " · ")
            return SearchHit(kind: t.kind, id: t.id, title: t.title, year: t.year,
                             posterUrl: t.posterUrl, info: info)
        }, notes: notes)
    }

    private func scheduleSearch() {
        searchTask?.cancel()
        let q = query.trimmingCharacters(in: .whitespaces)
        backToResults()
        guard !q.isEmpty else {
            results = []; searchError = nil; searching = false
            selection = rows.first
            loadHome(tab)
            return
        }
        searching = true
        searchTask = Task {
            try? await Task.sleep(for: .milliseconds(200))
            guard !Task.isCancelled else { return }
            do {
                let hits = try await api.search(q)
                guard !Task.isCancelled else { return }
                results = hits
                selection = hits.first
                searchError = nil
            } catch is CancellationError {
                return
            } catch {
                guard !Task.isCancelled else { return }
                results = []; selection = nil
                searchError = error.localizedDescription
            }
            searching = false
        }
    }

    func move(_ delta: Int) {
        if detailHit != nil {
            // Up from the first credit returns to the top of the page.
            let count = detailRows.count
            guard count > 0 else { return }
            let next = (detailRow ?? -1) + delta
            detailRow = next < 0 ? nil : min(count - 1, next)
            return
        }
        let rows = rows
        guard !rows.isEmpty else { return }
        let index = selection.flatMap { rows.firstIndex(of: $0) } ?? -1
        selection = rows[max(0, min(rows.count - 1, index + delta))]
    }

    /// Opens a result's detail, starting a fresh back stack.
    func open(_ hit: SearchHit) {
        selection = hit
        back = []
        load(hit)
    }

    /// Opens a row on the page in detail, keeping that page to go back to.
    func openRow(_ index: Int) {
        guard let hit = detailHit, let row = detailRows[safe: index] else { return }
        back.append(Page(hit: hit, detail: detail, error: detailError, row: index, kind: personKind,
                         listsUnread: personListsUnread))
        load(row)
    }

    /// Returns to the previous page, or to the results from the first one.
    func goBack() {
        guard let page = back.popLast() else {
            backToResults()
            return
        }
        if page.detail == nil {
            // It never finished loading, so try again.
            load(page.hit)
            return
        }
        detailTask?.cancel()
        resetPerson()
        viewing = nil
        peeking = false
        detailHit = page.hit
        detail = page.detail
        detailError = page.error
        personKind = page.kind
        personListsUnread = page.listsUnread
        detailRow = page.row
        pageID += 1
    }

    /// Fetches a row's summary from its page's detail when its list had none.
    /// A failure leaves the row without one.
    func fillSummary(_ hit: SearchHit) {
        guard hit.summary == nil, summaryAsked.insert(hit).inserted else { return }
        Task {
            guard let d = try? await api.detail(hit) else { return }
            switch d {
            case .title(let t):
                if let m = t.marks, marks[hit] == nil { marks[hit] = m }
                if let o = t.overview, !o.isEmpty { summaries[hit] = o }
            case .person(let p):
                let titles = p.knownForTitles.prefix(3)
                    .map { c in c.year.map { "\(c.title) (\($0))" } ?? c.title }
                    .joined(separator: ", ")
                let text = [p.knownFor, titles.isEmpty ? nil : "known for " + titles]
                    .compactMap { $0 }.joined(separator: " · ")
                if !text.isEmpty { summaries[hit] = text.prefix(1).uppercased() + text.dropFirst() }
            case .season, .episode, .credits: break
            }
        }
    }

    private func load(_ hit: SearchHit) {
        viewing = nil
        peeking = false
        detailTask?.cancel()
        resetPerson()
        tagDraft = ""
        detailHit = hit
        detail = nil
        detailError = nil
        detailRow = nil
        pageID += 1
        detailTask = Task {
            do {
                let d = try await api.detail(hit)
                guard !Task.isCancelled else { return }
                detail = d
                if case .title(let t) = d, let m = t.marks {
                    marks[hit] = m
                    if m.toWatch == true { loadKnownTags() }
                }
                if case .person(let p) = d { await loadListMarks(p) }
            } catch is CancellationError {
            } catch {
                guard !Task.isCancelled else { return }
                detailError = error.localizedDescription
            }
        }
    }

    /// Forgets the person page's filter, sort in flight and unread lists.
    private func resetPerson() {
        sortTask?.cancel()
        sortTask = nil
        personKind = nil
        personSorting = false
        personSortError = nil
        personListsUnread = []
    }

    /// Reads To watch and Favorites to mark the person's credits, as the web
    /// does: the person's page carries no marks. A list that can't be read
    /// leaves its marks unknown, and ⌘T or ⌘F reads the title's own first.
    private func loadListMarks(_ p: PersonDetail) async {
        async let toWatch = try? api.watchlist(sort: .added)
        async let favorites = try? api.favorites()
        let (w, f) = await (toWatch, favorites)
        guard !Task.isCancelled else { return }
        let key = { (e: ListEntry) in SearchHit(kind: e.kind, id: e.id) }
        let listed = w.map { Set($0.titles.map(key)) }
        let favored = f.map { Set($0.titles.map(key)) }
        for c in p.knownForTitles + p.filmography.flatMap(\.entries) {
            let hit = c.hit
            var m = marks[hit] ?? TitleMarks()
            if let listed { m.toWatch = listed.contains(hit) }
            if let favored { m.favorite = favored.contains(hit) }
            marks[hit] = m
        }
        personListsUnread = (listed == nil ? [.toWatch] : []) + (favored == nil ? [.favorite] : [])
    }

    /// Asks for the person in detail's credits in another order, keeping the
    /// current order on screen until it's back, and after a failure.
    func sortPerson(_ sort: FilmographySort) {
        guard let hit = detailHit, case .person(let p) = detail, p.sort != sort.rawValue else { return }
        sortTask?.cancel()
        personSorting = true
        personSortError = nil
        sortTask = Task {
            do {
                let sorted = try await api.person(p.id, sort: sort)
                guard !Task.isCancelled, detailHit == hit else { return }
                detail = .person(sorted)
                detailRow = nil
            } catch is CancellationError {
                return
            } catch {
                guard !Task.isCancelled else { return }
                personSortError = error.localizedDescription
            }
            personSorting = false
        }
    }

    func backToResults() {
        detailTask?.cancel()
        peeking = false
        resetPerson()
        detailHit = nil
        detail = nil
        detailError = nil
        detailRow = nil
        back = []
    }

    func openInBrowser(_ hit: SearchHit) {
        NSWorkspace.shared.open(api.webURL(hit))
        close()
    }

    /// Opens a link out, like a movie's showtimes, in the browser.
    func openLink(_ url: String) {
        guard let url = URL(string: url) else { return }
        NSWorkspace.shared.open(url)
        close()
    }

    /// The title a list shortcut applies to: the credit selected on a person's
    /// page, else the one in detail, else the selected result.
    var listTarget: SearchHit? {
        if case .person = detail, let row = detailRow.flatMap({ detailRows[safe: $0] }), row.isTitle { return row }
        return detailHit ?? selection
    }

    /// The episode ⌘W marks watched: the one selected on a season's or an
    /// episode's page, else the episode in detail.
    var watchedTarget: SearchHit? {
        let row = detailRow.flatMap { detailRows[safe: $0] }
        switch detail {
        case .season:
            return row?.kind == .episode ? row : nil
        case .episode(let e):
            if let row, row.kind == .episode { return row }
            return SearchHit(kind: .episode, id: e.showId, title: e.title, season: e.seasonNumber, episode: e.number)
        default:
            return nil
        }
    }

    /// The show's watched marks on the page in detail.
    private var pageWatched: WatchedMarks? {
        switch detail {
        case .title(let t): return t.watched
        case .season(let s): return s.watched
        case .episode(let e): return e.watched
        default: return nil
        }
    }

    /// An episode as a flash names it: "3. Title", or "Episode 3" from a pager.
    private func label(_ hit: SearchHit) -> String {
        let n = hit.episode ?? 0
        return hit.title.map { "\(n). \($0)" } ?? "Episode \(n)"
    }

    /// Flips a list for `hit`, a credit's button, or else the list shortcut's target.
    func toggle(_ list: ListAction, hit row: SearchHit? = nil) {
        if row == nil, list == .watched, let target = watchedTarget {
            toggleWatched(target)
            return
        }
        guard let hit = row ?? listTarget else { return }
        guard hit.isTitle else {
            show(hit.kind == .season || hit.kind == .episode ? "Go back to the show to change its lists"
                 : hit.kind == .credits ? "Go back to the title to change its lists"
                 : "Only movies and shows go on lists",
                 failed: true)
            return
        }
        guard !changing.contains(hit) else { return }
        changing.insert(hit)
        Task {
            defer { changing.remove(hit) }
            do {
                // Results don't carry list state, so learn it before flipping it.
                if marks[hit]?[list] == nil, case .title(let t) = try await api.detail(hit),
                   let m = t.marks {
                    marks[hit] = m
                }
                guard let current = marks[hit]?[list] else {
                    show("Couldn't read whether \(hit.displayName) is in \(list.name)", failed: true)
                    return
                }
                marks[hit] = try await api.setList(list, hit.kind, hit.id, !current)
                // Leaving To watch drops its tags on the server.
                if list == .toWatch, detailHit == hit, case .title(var t) = detail {
                    if current { t.sources = nil } else { loadKnownTags() }
                    detail = .title(t)
                }
                show("\(list.done(!current)): \(hit.displayName)", failed: false)
            } catch is CancellationError {
            } catch {
                show("Couldn't update \(list.name): \(error.localizedDescription)", failed: true)
            }
        }
    }

    /// Flips an episode's watched mark, then shows the show's marks the
    /// server answers with on every page of the show, here and behind.
    func toggleWatched(_ episode: SearchHit) {
        guard let season = episode.season, let number = episode.episode, !changing.contains(episode) else { return }
        let on = !(pageWatched?.episode(season, number) ?? false)
        changing.insert(episode)
        Task {
            defer { changing.remove(episode) }
            do {
                let marks = try await api.setEpisodeWatched(episode.id, season, number, on)
                applyWatched(marks, show: episode.id)
                show("\(on ? "Marked watched" : "Unmarked watched"): \(label(episode))", failed: false)
            } catch is CancellationError {
            } catch {
                show("Couldn't update Watched: \(error.localizedDescription)", failed: true)
            }
        }
    }

    /// Puts a show's new watched marks on its pages, in detail and behind.
    private func applyWatched(_ w: WatchedMarks, show id: Int) {
        func updated(_ d: Detail?) -> Detail? {
            switch d {
            case .title(var t) where t.kind == .tv && t.id == id: t.watched = w; return .title(t)
            case .season(var s) where s.showId == id: s.watched = w; return .season(s)
            case .episode(var e) where e.showId == id: e.watched = w; return .episode(e)
            default: return d
            }
        }
        detail = updated(detail)
        for i in back.indices { back[i].detail = updated(back[i].detail) }
        let showHit = SearchHit(kind: .tv, id: id)
        if marks[showHit] != nil { marks[showHit]?.watched = w.show }
    }

    /// Whether ⌘P can peek at the episode in detail: it isn't watched.
    var canPeek: Bool {
        if case .episode(let e) = detail { return !e.seen }
        return false
    }

    /// The page in detail's tags, when it's a title on To watch.
    var detailTags: [String]? {
        guard let hit = detailHit, case .title(let t) = detail, marks[hit]?.toWatch == true else { return nil }
        return t.sources ?? []
    }

    /// Tags used before that the page in detail doesn't have, to add with a click.
    var tagSuggestions: [String] {
        guard let tags = detailTags else { return [] }
        let has = Set(tags.map { $0.lowercased() })
        return Array(knownTags.filter { !has.contains($0.lowercased()) }.prefix(6))
    }

    private func loadKnownTags() {
        Task { if let tags = try? await api.watchlistSources() { knownTags = tags } }
    }

    /// Adds the tags typed in the field, separated by spaces.
    func addTypedTags() {
        let typed = tagDraft.split(whereSeparator: \.isWhitespace).map(String.init).filter { $0 != "#" }
        guard !typed.isEmpty, let tags = detailTags else { return }
        setTags(tags + typed, clearDraft: true)
    }

    func addTag(_ name: String) {
        guard let tags = detailTags else { return }
        setTags(tags + [name], clearDraft: false)
    }

    func removeTag(_ name: String) {
        guard let tags = detailTags else { return }
        setTags(tags.filter { $0 != name }, clearDraft: false)
    }

    /// Saves the page in detail's tags, then shows them as the server tidied them.
    private func setTags(_ tags: [String], clearDraft: Bool) {
        guard let hit = detailHit, !changing.contains(hit) else { return }
        changing.insert(hit)
        Task {
            defer { changing.remove(hit) }
            do {
                let saved = try await api.setSources(hit.kind, hit.id, tags)
                if detailHit == hit, case .title(var t) = detail {
                    t.sources = saved
                    detail = .title(t)
                }
                if clearDraft { tagDraft = "" }
                loadKnownTags()
                // To watch shows tags too, so read it again for next time.
                reloadHome(.toWatch)
            } catch is CancellationError {
            } catch {
                show("Couldn't save the tags: \(error.localizedDescription)", failed: true)
            }
        }
    }

    private func show(_ text: String, failed: Bool) {
        let f = Flash(text: text, failed: failed)
        flash = f
        flashTask?.cancel()
        flashTask = Task {
            try? await Task.sleep(for: .seconds(failed ? 6 : 3))
            guard !Task.isCancelled, flash == f else { return }
            flash = nil
        }
    }

    /// Opens an image full size; nothing when there's none.
    func view(_ url: String?, _ label: String) {
        guard let shown = url.flatMap(URL.init(string:)) else { return }
        viewing = FullImage(shown: shown, full: fullSize(shown), label: label)
    }

    /// What ⌘Y opens on the page in detail: the selected episode's still, else
    /// the page's poster, photo or still. An episode's still opens only while
    /// its spoilers show.
    var pageImage: (url: String, label: String)? {
        switch detail {
        case .title(let t): return t.posterUrl.map { ($0, "\(t.title) poster") }
        case .person(let p): return p.profileUrl.map { ($0, p.name) }
        case .season(let s):
            if let row = detailRow.flatMap({ detailRows[safe: $0] }), row.kind == .episode {
                let e = s.episodes.first { $0.number == row.episode }
                return e?.stillUrl.map { ($0, "\(e!.title) still") }
            }
            return s.posterUrl.map { ($0, "\(s.name) poster") }
        case .episode(let e):
            return e.seen || peeking ? e.stillUrl.map { ($0, "\(e.title) still") } : nil
        case .credits, nil: return nil
        }
    }

    // Keys, routed from the panel's key monitor. Each returns whether it was handled.

    func handleReturn(command: Bool) -> Bool {
        if let hit = detailHit {
            // ⏎ opens the selected credit's page here; with none selected, and
            // with ⌘, it opens the selected credit, or the page itself, on the web.
            if let index = detailRow, let row = detailRows[safe: index] {
                if command { openInBrowser(row) } else { openRow(index) }
            } else {
                openInBrowser(hit)
            }
            return true
        }
        guard let hit = selection else { return false }
        if command { openInBrowser(hit) } else { open(hit) }
        return true
    }

    func handleEscape() {
        if detailHit != nil { goBack() } else { close() }
    }
}

enum HomeTab: CaseIterable {
    case recent, toWatch, nowPlaying

    var name: String {
        switch self {
        case .recent: return "Recently viewed"
        case .toWatch: return "To watch"
        case .nowPlaying: return "Now playing"
        }
    }

    var icon: String {
        switch self {
        case .recent: return "clock"
        case .toWatch: return "bookmark"
        case .nowPlaying: return "ticket"
        }
    }

    var empty: String {
        switch self {
        case .recent: return "Nothing viewed yet"
        case .toWatch: return "Nothing on To watch"
        case .nowPlaying: return "Nothing playing in theaters"
        }
    }
}

/// Which To watch titles the tab shows, as the web's filters choose them.
struct ToWatchFilter: Equatable {
    /// Movies or TV; nil for both.
    var kind: Kind?
    var onMyServices = false
    /// `out_now` or `coming_soon`; nil for any.
    var release: String?
    /// A recommended-by tag, matched ignoring case; nil for any.
    var tag: String?

    func keeps(_ t: ListEntry) -> Bool {
        (kind == nil || t.kind == kind)
            && (!onMyServices || !(t.availability?.services ?? []).isEmpty)
            && (release == nil || t.releaseStatus == release)
            && (tag == nil || (t.sources ?? []).contains { $0.lowercased() == tag!.lowercased() })
    }

    func with<T>(_ key: WritableKeyPath<ToWatchFilter, T>, _ value: T) -> ToWatchFilter {
        var copy = self
        copy[keyPath: key] = value
        return copy
    }
}

/// A tab's list as last loaded, with notes on anything it's missing.
struct HomeList {
    let hits: [SearchHit]
    var notes: [String] = []
    let loadedAt = Date()
}

/// "2026-11-06" or an ISO timestamp as "Nov 6, 2026".
/// The original upload behind a sized TMDB image URL
/// (`…/t/p/w342/x.jpg` to `…/t/p/original/x.jpg`), as the web's lightbox opens.
func fullSize(_ url: URL) -> URL {
    let sized = url.deletingLastPathComponent()
    guard sized.deletingLastPathComponent().lastPathComponent == "p" else { return url }
    return sized.deletingLastPathComponent().appendingPathComponent("original").appendingPathComponent(url.lastPathComponent)
}

func shortDate(_ iso: String) -> String {
    let f = ISO8601DateFormatter()
    f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    let day = ISO8601DateFormatter()
    day.formatOptions = [.withFullDate]
    day.timeZone = .current
    let date = f.date(from: iso) ?? ISO8601DateFormatter().date(from: iso) ?? day.date(from: iso)
    return date?.formatted(date: .abbreviated, time: .omitted) ?? iso
}

extension Collection {
    subscript(safe index: Index) -> Element? { indices.contains(index) ? self[index] : nil }
}
