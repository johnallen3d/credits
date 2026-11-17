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

    /// Bumped each time the panel opens, so the view refocuses the field.
    var shown = 0
    var close: () -> Void = {}

    private var searchTask: Task<Void, Never>?
    private var detailTask: Task<Void, Never>?

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
