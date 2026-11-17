import AppKit
import SwiftUI

/// The menu bar menu's My Services and About windows: ordinary titled windows,
/// kept and brought back, their content loaded afresh each time, in the launcher's theme.
@MainActor
final class SideWindows {
    private var windows: [String: NSWindow] = [:]

    func show<Content: View>(_ key: String, title: String, size: NSSize, @ViewBuilder content: () -> Content) {
        let window = windows[key] ?? {
            let window = NSWindow(contentRect: NSRect(origin: .zero, size: size),
                                  styleMask: [.titled, .closable, .miniaturizable, .resizable],
                                  backing: .buffered, defer: false)
            window.title = title
            window.isReleasedWhenClosed = false
            window.center()
            windows[key] = window
            return window
        }()
        // New content each time, so it loads afresh.
        window.contentView = NSHostingView(rootView: content().themed())
        window.appearance = ThemeStore.shared.name.appearance
        NSApp.activate(ignoringOtherApps: true)
        window.makeKeyAndOrderFront(nil)
    }

    /// Follows a theme change in any open window.
    func applyTheme() {
        for window in windows.values { window.appearance = ThemeStore.shared.name.appearance }
    }
}

private extension View {
    func themed() -> some View {
        foregroundStyle(Theme.fg)
            .tint(Theme.accent)
            .background(Theme.bg)
    }
}

/// Choose the streaming services you subscribe to, as the web's services page
/// does: title pages highlight them and To watch can filter by them. Each box
/// saves at once, in order, so the last click wins; until its reply lands a
/// box shows what was asked.
struct ServicesView: View {
    let api: CreditsAPI
    @State private var page: ServicesPage?
    @State private var loadError: String?
    @State private var saveError: String?
    @State private var asked: [Int: Bool] = [:]
    @State private var saving: Task<Void, Never>?

    var body: some View {
        Group {
            if let page {
                list(page)
            } else if let loadError {
                VStack(spacing: 12) {
                    Notice(icon: "exclamationmark.triangle", text: "Couldn't load your services: \(loadError)", tint: Theme.love)
                        .frame(maxHeight: 160)
                    Button("Try Again") { Task { await load() } }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .frame(minWidth: 380, minHeight: 360)
        .task { await load() }
    }

    private func load() async {
        loadError = nil
        do { page = try await api.services() } catch { loadError = error.localizedDescription }
    }

    private func list(_ page: ServicesPage) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 10) {
                Text("My services").font(.title2.bold()) + Text("  in \(page.region)").foregroundStyle(Theme.muted)
                Text(page.chosen.isEmpty ? "No services chosen yet."
                     : "Your services: " + page.chosen.map(\.name).joined(separator: ", ") + ".")
                if let saveError {
                    note("Couldn't save your services (\(saveError)). Please try again.", bad: true)
                }
                if page.state == "unavailable" {
                    note("TMDB's list of services couldn't be loaded right now"
                         + (page.chosen.isEmpty ? "." : ", so only your current services are listed. You can still remove them."), bad: true)
                } else if page.state == "stale" {
                    note("Couldn't refresh TMDB's list of services"
                         + (page.fetchedAt.map { "; showing the list from \(shortDate($0))." } ?? "."), bad: true)
                }
                if !page.complete && page.state != "unavailable" {
                    note("Only part of TMDB's list of services could be loaded, so some may be missing.", bad: false)
                }
                ForEach(["mine", "popular", "more"], id: \.self) { group in
                    let options = page.options.filter { $0.group == group }
                    if !options.isEmpty {
                        Section(heading: group == "mine" ? "Your services" : group == "popular" ? "Popular" : "More services") {
                            ForEach(options) { row($0) }
                        }
                        .padding(.top, 6)
                    }
                }
                if page.state != "unavailable" {
                    Text("Services from JustWatch, via TMDB. Channels sold through another service, like Amazon or Roku channels, aren't listed; choose the service itself.")
                        .font(.caption).foregroundStyle(Theme.muted)
                        .padding(.top, 6)
                }
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
            .fixedSize(horizontal: false, vertical: true)
        }
    }

    private func row(_ option: ServiceChoice) -> some View {
        Toggle(isOn: Binding(get: { asked[option.id] ?? option.chosen ?? false },
                             set: { choose(option.id, $0) })) {
            HStack(spacing: 8) {
                Group {
                    if let url = option.logoUrl.flatMap(URL.init(string:)) {
                        AsyncImage(url: url) { $0.resizable().aspectRatio(contentMode: .fill) } placeholder: { Theme.line }
                    } else {
                        Theme.line
                    }
                }
                .frame(width: 24, height: 24)
                .clipShape(RoundedRectangle(cornerRadius: 5))
                Text(option.name)
            }
        }
        .toggleStyle(.checkbox)
    }

    private func choose(_ id: Int, _ on: Bool) {
        asked[id] = on
        let previous = saving
        saving = Task {
            await previous?.value
            do {
                page = try await api.chooseService(id, on)
                saveError = nil
            } catch {
                saveError = error.localizedDescription
            }
            if asked[id] == on { asked[id] = nil }
        }
    }

    private func note(_ text: String, bad: Bool) -> some View {
        Label(text, systemImage: bad ? "exclamationmark.triangle" : "info.circle")
            .font(.callout)
            .foregroundStyle(bad ? Theme.love : Theme.muted)
            .fixedSize(horizontal: false, vertical: true)
    }
}

/// What credits is, where its data comes from (with the notice TMDB's terms
/// ask for) and how Home Assistant is doing, as the web's About page shows.
/// The credits never wait on the server; only the Home Assistant line does.
struct AboutView: View {
    let api: CreditsAPI
    /// Opens Seen on Apple TV in the launcher.
    let openPlayback: () -> Void
    @State private var live: LiveStatus?
    @State private var loadError: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 10) {
                Image(nsImage: NSApp.applicationIconImage).resizable().frame(width: 48, height: 48)
                VStack(alignment: .leading) {
                    Text("Credits").font(.title2.bold())
                    if let version = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String {
                        Text("Version \(version)").foregroundStyle(Theme.muted)
                    }
                }
            }
            Text("credits looks up movies, TV shows and people, and shows who made them.")
            Section(heading: "Data sources") {
                Link("TMDB", destination: URL(string: "https://www.themoviedb.org/")!).font(.headline).foregroundStyle(Theme.accent)
                Text("This product uses the TMDB API but is not endorsed or certified by TMDB.")
                Text("Ratings via the [OMDb API](https://www.omdbapi.com/).")
                Text("Episode ratings OMDb lacks: information courtesy of [IMDb](https://www.imdb.com). Used with permission.")
                Text("Where to watch data by [JustWatch](https://www.justwatch.com/), via TMDB.")
            }
            .padding(.top, 6)
            Section(heading: "Home Assistant") { homeAssistant }
                .padding(.top, 6)
            Spacer(minLength: 0)
        }
        .fixedSize(horizontal: false, vertical: true)
        .padding(20)
        .frame(minWidth: 420, maxWidth: .infinity, minHeight: 360, maxHeight: .infinity, alignment: .topLeading)
        .task { await load() }
    }

    private func load() async {
        loadError = nil
        do { live = try await api.playback().live } catch { loadError = error.localizedDescription }
    }

    @ViewBuilder private var homeAssistant: some View {
        if let live {
            let since = live.since.map { " since \(shortTime($0))" } ?? ""
            switch live.state {
            case "not_configured":
                Text("Not set up, so episodes and movies aren't marked from the Apple TV. Set HA_URL and HA_TOKEN on the server and restart it.")
                    .foregroundStyle(Theme.muted)
            case "connecting":
                Text("Connecting…").foregroundStyle(Theme.muted)
            case "connected":
                Text("Connected\(since). Episodes and movies played on the Apple TV are marked as they finish.")
            default:
                Text("Disconnected\(since), retrying: \(live.error ?? "unknown error").").foregroundStyle(Theme.love)
            }
            if live.state != "not_configured" {
                Button("Seen on Apple TV", action: openPlayback).buttonStyle(.link).foregroundStyle(Theme.accent)
            }
        } else if let loadError {
            Text("Couldn't load Home Assistant's status: \(loadError)").foregroundStyle(Theme.love)
            Button("Try Again") { Task { await load() } }
        } else {
            ProgressView().controlSize(.small)
        }
    }
}
