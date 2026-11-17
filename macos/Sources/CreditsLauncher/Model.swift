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

    var detailHit: SearchHit?
    var detail: Detail?
    var detailError: String?

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

    func panelOpened() {
        shown += 1
    }

    private func scheduleSearch() {
        searchTask?.cancel()
        let q = query.trimmingCharacters(in: .whitespaces)
        backToResults()
        guard !q.isEmpty else {
            results = []; selection = nil; searchError = nil; searching = false
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
        guard detailHit == nil, !results.isEmpty else { return }
        let index = selection.flatMap { results.firstIndex(of: $0) } ?? -1
        selection = results[max(0, min(results.count - 1, index + delta))]
    }

    func open(_ hit: SearchHit) {
        detailTask?.cancel()
        selection = hit
        detailHit = hit
        detail = nil
        detailError = nil
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
            openInBrowser(hit.kind, hit.id)
            return true
        }
        guard let hit = selection else { return false }
        if command { openInBrowser(hit.kind, hit.id) } else { open(hit) }
        return true
    }

    func handleEscape() {
        if detailHit != nil { backToResults() } else { close() }
    }
}
