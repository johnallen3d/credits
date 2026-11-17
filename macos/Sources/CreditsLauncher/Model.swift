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

    var showingHome: Bool { query.trimmingCharacters(in: .whitespaces).isEmpty }
    /// What ↑↓ and ⏎ act on: the search results, or the tab's list.
    var rows: [SearchHit] { showingHome ? home[tab]?.hits ?? [] : results }

    /// The page in detail: a result, or a credit opened from another page.
    var detailHit: SearchHit?
    var detail: Detail?
    var detailError: String?
    /// The selected credit on the page in detail, an index into `detail.rows`.
    var detailRow: Int?
    /// The pages behind the one in detail, oldest first.
    var back: [Page] = []
    /// Bumped on every page change, so the view starts each page at its top.
    var pageID = 0

    struct Page {
        let hit: SearchHit
        let detail: Detail?
        let error: String?
        let row: Int?
    }

    /// List state per title, from its detail or the last change.
    var marks: [SearchHit: TitleMarks] = [:]
    /// Titles with a list change in flight.
    var changing: Set<SearchHit> = []
    /// A one-line result of the last list action.
    var flash: Flash?

    struct Flash: Equatable {
        let text: String
        let failed: Bool
        let id = UUID()
    }

    /// Bumped each time the panel opens, so the view refocuses the field.
    var shown = 0
    var close: () -> Void = {}

    private var searchTask: Task<Void, Never>?
    private var detailTask: Task<Void, Never>?
    private var flashTask: Task<Void, Never>?
    private var homeTasks: [HomeTab: Task<Void, Never>] = [:]

    func panelOpened() {
        shown += 1
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

    /// Reloads a tab's list, keeping the saved one on screen until it's back.
    private func loadHome(_ tab: HomeTab) {
        guard homeTasks[tab] == nil else { return }
        homeLoading.insert(tab)
        homeTasks[tab] = Task {
            defer { homeLoading.remove(tab); homeTasks[tab] = nil }
            do {
                let list = try await fetch(tab)
                home[tab] = list
                homeError[tab] = nil
            } catch {
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
            let w = try await api.watchlist()
            for t in w.titles {
                if let m = t.marks { marks[SearchHit(kind: t.kind, id: t.id)] = m }
            }
            var notes: [String] = []
            if let s = w.services {
                if s.state == "unreadable" { notes.append("Your streaming services couldn't be read") }
                if let since = s.staleSince { notes.append("Services saved from \(shortDate(since)): TMDB isn't responding") }
                if s.unknown > 0 {
                    notes.append("Services couldn't be loaded for \(s.unknown) title\(s.unknown == 1 ? "" : "s")")
                }
            }
            return HomeList(hits: w.titles.map { t in
                let services = (t.availability?.services ?? []).map(\.name)
                let from = t.source.map { "from \($0)" }
                let info = ([t.year].compactMap { $0 } + services + [from].compactMap { $0 })
                    .joined(separator: " · ")
                return SearchHit(kind: t.kind, id: t.id, title: t.title, year: t.year,
                                 posterUrl: t.posterUrl, info: info)
            }, notes: notes)
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
            let count = detail?.rows.count ?? 0
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

    /// Opens a credit on the page in detail, keeping that page to go back to.
    func openRow(_ index: Int) {
        guard let hit = detailHit, let row = detail?.rows[safe: index] else { return }
        back.append(Page(hit: hit, detail: detail, error: detailError, row: index))
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
        detailHit = page.hit
        detail = page.detail
        detailError = page.error
        detailRow = page.row
        pageID += 1
    }

    private func load(_ hit: SearchHit) {
        detailTask?.cancel()
        detailHit = hit
        detail = nil
        detailError = nil
        detailRow = nil
        pageID += 1
        detailTask = Task {
            do {
                let d = try await api.detail(hit.kind, hit.id)
                guard !Task.isCancelled else { return }
                detail = d
                if case .title(let t) = d, let m = t.marks { marks[hit] = m }
            } catch is CancellationError {
            } catch {
                guard !Task.isCancelled else { return }
                detailError = error.localizedDescription
            }
        }
    }

    func backToResults() {
        detailTask?.cancel()
        detailHit = nil
        detail = nil
        detailError = nil
        detailRow = nil
        back = []
    }

    func openInBrowser(_ kind: Kind, _ id: Int) {
        NSWorkspace.shared.open(api.webURL(kind, id))
        close()
    }

    /// The title a list shortcut applies to: the one in detail, else the selected result.
    var listTarget: SearchHit? { detailHit ?? selection }

    func toggle(_ list: ListAction) {
        guard let hit = listTarget else { return }
        guard hit.kind != .person else {
            show("Only movies and shows go on lists", failed: true)
            return
        }
        guard !changing.contains(hit) else { return }
        changing.insert(hit)
        Task {
            defer { changing.remove(hit) }
            do {
                // Results don't carry list state, so learn it before flipping it.
                if marks[hit]?[list] == nil, case .title(let t) = try await api.detail(hit.kind, hit.id),
                   let m = t.marks {
                    marks[hit] = m
                }
                guard let current = marks[hit]?[list] else {
                    show("Couldn't read whether \(hit.displayName) is in \(list.name)", failed: true)
                    return
                }
                marks[hit] = try await api.setList(list, hit.kind, hit.id, !current)
                show("\(list.done(!current)): \(hit.displayName)", failed: false)
            } catch is CancellationError {
            } catch {
                show("Couldn't update \(list.name): \(error.localizedDescription)", failed: true)
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

    // Keys, routed from the panel's key monitor. Each returns whether it was handled.

    func handleReturn(command: Bool) -> Bool {
        if let hit = detailHit {
            // ⏎ opens the selected credit's page here; with none selected, and
            // with ⌘, it opens the selected credit, or the page itself, on the web.
            if let index = detailRow, let row = detail?.rows[safe: index] {
                if command { openInBrowser(row.kind, row.id) } else { openRow(index) }
            } else {
                openInBrowser(hit.kind, hit.id)
            }
            return true
        }
        guard let hit = selection else { return false }
        if command { openInBrowser(hit.kind, hit.id) } else { open(hit) }
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

/// A tab's list as last loaded, with notes on anything it's missing.
struct HomeList {
    let hits: [SearchHit]
    var notes: [String] = []
    let loadedAt = Date()
}

/// "2026-11-06" or an ISO timestamp as "Nov 6, 2026".
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
