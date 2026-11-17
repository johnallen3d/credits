import SwiftUI

struct LauncherView: View {
    @Bindable var model: LauncherModel
    @FocusState private var fieldFocused: Bool

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 10) {
                Image(systemName: model.detailHit == nil ? "magnifyingglass" : "chevron.left")
                    .font(.title2)
                    .foregroundStyle(Theme.muted)
                    .onTapGesture { model.goBack() }
                TextField("Search movies, TV and people", text: $model.query)
                    .textFieldStyle(.plain)
                    .font(.system(size: 22))
                    .focused($fieldFocused)
                if model.searching {
                    ProgressView().controlSize(.small)
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 14)

            Rule()

            Group {
                if let hit = model.detailHit {
                    DetailContainer(model: model, hit: hit)
                } else if model.showingHome {
                    HomeView(model: model)
                } else {
                    ResultsView(model: model)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .overlay(alignment: .bottom) {
                if let flash = model.flash {
                    FlashView(flash: flash).padding(10)
                }
            }
            .animation(.easeOut(duration: 0.15), value: model.flash)

            Rule()
            FooterView(inDetail: model.detailHit != nil,
                       home: model.showingHome,
                       rowSelected: model.detailRow != nil,
                       canGoBack: !model.back.isEmpty,
                       lists: model.listTarget.map { $0.kind != .person } ?? false)
        }
        .frame(width: 720, height: 480)
        .foregroundStyle(Theme.fg)
        .tint(Theme.accent)
        .background(Theme.bg)
        .clipShape(RoundedRectangle(cornerRadius: 14))
        .onChange(of: model.shown, initial: true) { fieldFocused = true }
    }
}

struct ResultsView: View {
    let model: LauncherModel

    var body: some View {
        if let error = model.searchError {
            Notice(icon: "exclamationmark.triangle", text: error, tint: Theme.love)
        } else if model.results.isEmpty {
            if model.searching {
                Notice(icon: "film", text: "Type to search")
            } else {
                Notice(icon: "questionmark.circle", text: "No matches for “\(model.query)”")
            }
        } else {
            HitList(model: model, hits: model.results)
        }
    }
}

/// With no query: Recently viewed, To watch and Now playing as tabs.
struct HomeView: View {
    let model: LauncherModel

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 4) {
                ForEach(HomeTab.allCases, id: \.self) { tab in
                    Label(tab.name, systemImage: tab.icon)
                        .font(.callout)
                        .foregroundStyle(tab == model.tab ? Theme.fg : Theme.muted)
                        .padding(.horizontal, 10).padding(.vertical, 4)
                        .background(tab == model.tab ? Theme.line : .clear, in: Capsule())
                        .contentShape(Capsule())
                        .onTapGesture { model.selectTab(tab) }
                }
                Spacer()
                if model.homeLoading.contains(model.tab) && model.home[model.tab] != nil {
                    ProgressView().controlSize(.small)
                }
                Text("⇥ Next list").font(.caption).foregroundStyle(Theme.subtle)
            }
            .padding(.horizontal, 10)
            .padding(.top, 8)

            VStack(spacing: 0) { content }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    @ViewBuilder private var content: some View {
        let list = model.home[model.tab]
        let error = model.homeError[model.tab]
        if let list {
            VStack(alignment: .leading, spacing: 3) {
                if let error {
                    note("Couldn't refresh (\(error)); showing the list from "
                         + list.loadedAt.formatted(date: .omitted, time: .shortened),
                         icon: "exclamationmark.triangle", tint: Theme.love)
                }
                ForEach(list.notes, id: \.self) { note($0, icon: "clock.arrow.circlepath", tint: Theme.movie) }
            }
            .padding(.horizontal, 16)
            .padding(.top, error != nil || !list.notes.isEmpty ? 8 : 0)
            if list.hits.isEmpty {
                Notice(icon: model.tab.icon, text: model.tab.empty)
            } else {
                HitList(model: model, hits: list.hits).id(model.tab)
            }
        } else if let error {
            Notice(icon: "exclamationmark.triangle", text: "Couldn't load \(model.tab.name): \(error)", tint: Theme.love)
        } else {
            ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    private func note(_ text: String, icon: String, tint: Color) -> some View {
        Label(text, systemImage: icon)
            .font(.callout)
            .foregroundStyle(tint)
            .lineLimit(2)
            .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// Search results or a list's entries, selectable with ↑↓.
struct HitList: View {
    let model: LauncherModel
    let hits: [SearchHit]

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(spacing: 8) {
                    ForEach(hits) { hit in
                        ResultRow(hit: hit, summary: hit.summary ?? model.summaries[hit],
                                  selected: hit == model.selection, marks: model.marks[hit])
                            .id(hit)
                            .onAppear { model.fillSummary(hit) }
                            .onTapGesture { model.open(hit) }
                    }
                }
                .padding(10)
            }
            .onChange(of: model.selection) { _, hit in
                if let hit { proxy.scrollTo(hit) }
            }
        }
    }
}

/// A hit as a card, laid out like the web's search results: poster, type badge,
/// name with year, and a short summary.
struct ResultRow: View {
    let hit: SearchHit
    let summary: String?
    let selected: Bool
    let marks: TitleMarks?

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Poster(url: hit.imageUrl, width: 44, kind: hit.kind)
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(hit.kindLabel.uppercased())
                        .font(.caption2.bold()).kerning(0.5)
                        .foregroundStyle(Theme.color(for: hit.kind))
                    if hit.inTheaters == true {
                        Text("IN THEATERS")
                            .font(.caption2.bold()).kerning(0.5)
                            .foregroundStyle(Theme.accent)
                            .padding(.horizontal, 4)
                            .overlay(RoundedRectangle(cornerRadius: 3).stroke(Theme.accent))
                    }
                }
                (Text(hit.displayName).font(.headline)
                    + Text(hit.year.map { "  (\($0))" } ?? "").foregroundColor(Theme.muted))
                    .lineLimit(1)
                if let summary {
                    Text(summary)
                        .font(.callout)
                        .foregroundStyle(Theme.muted)
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer(minLength: 0)
            if let marks {
                HStack(spacing: 6) {
                    ForEach(ListAction.allCases, id: \.self) { list in
                        if marks[list] == true {
                            Image(systemName: list.icon + ".fill").foregroundStyle(Theme.accent)
                        }
                    }
                }
                .font(.callout)
            }
        }
        .padding(8)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Theme.card, in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8)
            .stroke(selected ? Theme.accent : Theme.line, lineWidth: selected ? 2 : 1))
        .contentShape(Rectangle())
    }
}

struct DetailContainer: View {
    let model: LauncherModel
    let hit: SearchHit

    var body: some View {
        if let error = model.detailError {
            Notice(icon: "exclamationmark.triangle", text: error, tint: Theme.love)
        } else if let detail = model.detail {
            let credits = Credits(selected: model.detailRow, open: { model.openRow($0) })
            ScrollViewReader { proxy in
                ScrollView {
                    Group {
                        switch detail {
                        case .title(let t):
                            TitleDetailView(title: t, marks: model.marks[hit] ?? t.marks,
                                            changing: model.changing.contains(hit), credits: credits)
                        case .person(let p): PersonDetailView(person: p, credits: credits)
                        }
                    }
                    .padding(18)
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .onChange(of: model.detailRow, initial: true) { _, row in
                    if let row { proxy.scrollTo(Credits.id(row)) }
                }
            }
            .id(model.pageID)
        } else {
            ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }
}

/// The credits on a detail page: which one is selected, and how to open one.
/// Rows are numbered as in `Detail.rows`.
struct Credits {
    let selected: Int?
    let open: (Int) -> Void

    static func id(_ row: Int) -> String { "credit-\(row)" }

    /// A credit's row, highlighted when selected, opening its page on click.
    func row(_ index: Int, @ViewBuilder _ content: () -> some View) -> some View {
        content()
            .padding(.horizontal, 5).padding(.vertical, 1)
            .background(index == selected ? Theme.line : .clear, in: RoundedRectangle(cornerRadius: 5))
            .padding(.horizontal, -5).padding(.vertical, -1)
            .contentShape(Rectangle())
            .onTapGesture { open(index) }
            .id(Self.id(index))
    }
}

struct TitleDetailView: View {
    let title: TitleDetail
    let marks: TitleMarks?
    let changing: Bool
    let credits: Credits

    /// Each key crew role with the row number of its first person.
    private var crew: [(role: KeyCrew, first: Int)] {
        var next = 0
        return title.keyCrew.map { role in
            defer { next += role.people.count }
            return (role, next)
        }
    }

    private var facts: String {
        var parts: [String] = []
        if let year = title.year { parts.append(year) }
        parts.append(title.kind == .tv ? "TV" : "Movie")
        if let cert = title.releases?.certification, !cert.isEmpty { parts.append(cert) }
        if let minutes = title.runtimeMinutes { parts.append("\(minutes / 60)h \(minutes % 60)m") }
        if let seasons = title.seasons {
            parts.append("\(seasons) season\(seasons == 1 ? "" : "s")")
        }
        if title.kind == .tv, let status = title.status { parts.append(status) }
        if !title.genres.isEmpty { parts.append(title.genres.joined(separator: ", ")) }
        return parts.joined(separator: " · ")
    }

    private var ratings: [Rating] {
        guard let r = title.ratings else { return [] }
        return (r.preferred.map { [$0] } ?? []) + r.others
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(alignment: .top, spacing: 16) {
                Poster(url: title.posterUrl, width: 120, kind: title.kind)
                VStack(alignment: .leading, spacing: 8) {
                    Text(title.title).font(.title.bold())
                    Text(facts).foregroundStyle(Theme.muted)
                    ListMarksView(marks: marks, changing: changing)
                    // Only while on To watch, since a toggle updates just the marks.
                    if let sources = title.sources, !sources.isEmpty, marks?.toWatch == true {
                        Text("Recommended by " + sources.map { "#\($0)" }.joined(separator: " "))
                            .foregroundStyle(Theme.muted)
                    }
                    if !ratings.isEmpty {
                        HStack {
                            ForEach(ratings, id: \.self) { r in
                                Text(r.label)
                                    .font(.callout.bold())
                                    .padding(.horizontal, 8).padding(.vertical, 3)
                                    .background(Theme.line, in: Capsule())
                            }
                        }
                    } else if title.ratings?.state == "unavailable" {
                        Text("Ratings unavailable").font(.callout).foregroundStyle(Theme.muted)
                    }
                    if let tagline = title.tagline, !tagline.isEmpty {
                        Text(tagline).italic().foregroundStyle(Theme.muted)
                    }
                    if let overview = title.overview, !overview.isEmpty {
                        Text(overview).fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
            StaleNotice(metadata: title.metadata)
            WhereToWatchView(whereToWatch: title.whereToWatch)
            ForEach(crew, id: \.first) { crew in
                Section(heading: crewHeading(crew.role.role)) {
                    FlowLayout {
                        ForEach(Array(crew.role.people.enumerated()), id: \.offset) { i, person in
                            HStack(spacing: 0) {
                                credits.row(crew.first + i) { Text(person.name) }
                                if i < crew.role.people.count - 1 {
                                    // Wider than a selected name's highlight spills.
                                    Text("·").foregroundStyle(Theme.muted).padding(.horizontal, 8)
                                }
                            }
                        }
                    }
                }
            }
            if !title.cast.isEmpty {
                let first = title.keyCrew.reduce(0) { $0 + $1.people.count }
                Section(heading: "Cast") {
                    VStack(alignment: .leading, spacing: 3) {
                        ForEach(Array(title.cast.prefix(Detail.castShown).enumerated()), id: \.offset) { i, c in
                            credits.row(first + i) {
                                HStack(spacing: 4) {
                                    Text(c.name)
                                    if let role = c.role, !role.isEmpty {
                                        Text("as \(role)").foregroundStyle(Theme.muted)
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    private func crewHeading(_ role: String) -> String {
        switch role {
        case "creator": return "Created by"
        case "director": return "Directed by"
        case "writer": return "Written by"
        default: return role.capitalized
        }
    }
}

/// To watch, Favorites and Watched, each on or off, with its shortcut.
struct ListMarksView: View {
    let marks: TitleMarks?
    let changing: Bool

    var body: some View {
        HStack(spacing: 6) {
            ForEach(ListAction.allCases, id: \.self) { list in
                chip(list, marks?[list])
            }
            if changing { ProgressView().controlSize(.small) }
        }
    }

    private func chip(_ list: ListAction, _ on: Bool?) -> some View {
        HStack(spacing: 5) {
            Image(systemName: on == nil ? "questionmark" : on! ? list.icon + ".fill" : list.icon)
            Text(list.name)
            Text("⌘" + list.key.uppercased()).foregroundStyle(Theme.subtle)
        }
        .font(.callout)
        .foregroundStyle(on == true ? Theme.accent : Theme.muted)
        .padding(.horizontal, 8).padding(.vertical, 3)
        .overlay(Capsule().stroke(on == true ? Theme.accent : Theme.line))
        .help(on == nil ? "\(list.name): couldn't read" : "")
    }
}

struct FlashView: View {
    let flash: LauncherModel.Flash

    var body: some View {
        Label(flash.text, systemImage: flash.failed ? "exclamationmark.triangle.fill" : "checkmark.circle.fill")
            .font(.callout)
            .lineLimit(2)
            .foregroundStyle(flash.failed ? Theme.love : Theme.fg)
            .padding(.horizontal, 12).padding(.vertical, 7)
            .background(Theme.line, in: RoundedRectangle(cornerRadius: 8))
            .shadow(radius: 6)
            .transition(.opacity)
    }
}

struct WhereToWatchView: View {
    let whereToWatch: WhereToWatch?

    var body: some View {
        if let w = whereToWatch {
            Section(heading: "Where to watch") {
                if w.state == "unavailable" {
                    Text("Where to watch is unavailable right now").foregroundStyle(Theme.muted)
                } else if w.offers.isEmpty {
                    Text("Not available to watch in your region").foregroundStyle(Theme.muted)
                } else {
                    VStack(alignment: .leading, spacing: 3) {
                        ForEach(w.offers, id: \.self) { group in
                            HStack(alignment: .firstTextBaseline, spacing: 6) {
                                Text(group.kind.capitalized)
                                    .foregroundStyle(Theme.muted)
                                    .frame(width: 56, alignment: .leading)
                                group.providers.enumerated().reduce(Text("")) { text, item in
                                    let p = item.element
                                    let name = p.mine ? Text(p.name).bold().foregroundColor(Theme.accent) : Text(p.name)
                                    return text + Text(item.offset == 0 ? "" : ", ") + name
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

struct PersonDetailView: View {
    let person: PersonDetail
    let credits: Credits

    private var born: String? {
        let parts = [person.birthday.map { "Born \($0)" }, person.placeOfBirth].compactMap { $0 }
        let base = parts.isEmpty ? nil : parts.joined(separator: " in ")
        if let died = person.deathday { return [base, "Died \(died)"].compactMap { $0 }.joined(separator: " · ") }
        return base
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(alignment: .top, spacing: 16) {
                Poster(url: person.profileUrl, width: 120, kind: .person)
                VStack(alignment: .leading, spacing: 8) {
                    Text(person.name).font(.title.bold())
                    if let knownFor = person.knownFor { Text(knownFor).foregroundStyle(Theme.muted) }
                    if let born { Text(born).foregroundStyle(Theme.muted) }
                    if let bio = person.biography, !bio.isEmpty {
                        Text(bio).lineLimit(9).fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
            StaleNotice(metadata: person.metadata)
            if !person.knownForTitles.isEmpty {
                Section(heading: "Known for") {
                    VStack(alignment: .leading, spacing: 3) {
                        ForEach(Array(person.knownForTitles.enumerated()), id: \.offset) { i, c in
                            credits.row(i) {
                                HStack(spacing: 4) {
                                    Image(systemName: c.kind == .tv ? "tv" : "film")
                                        .font(.caption)
                                        .foregroundStyle(Theme.color(for: c.kind))
                                    Text(c.title)
                                    if let year = c.year { Text("(\(year))").foregroundStyle(Theme.muted) }
                                    if let role = c.role, !role.isEmpty {
                                        Text("· \(role)").foregroundStyle(Theme.muted)
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Lays its children out in rows, wrapping to the width it's given.
struct FlowLayout: Layout {
    var spacing: CGFloat = 0
    var lineSpacing: CGFloat = 3

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        arrange(proposal.width ?? .infinity, subviews).size
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        for (subview, point) in zip(subviews, arrange(bounds.width, subviews).points) {
            subview.place(at: CGPoint(x: bounds.minX + point.x, y: bounds.minY + point.y), proposal: .unspecified)
        }
    }

    private func arrange(_ width: CGFloat, _ subviews: Subviews) -> (points: [CGPoint], size: CGSize) {
        var points: [CGPoint] = []
        var x: CGFloat = 0, y: CGFloat = 0, lineHeight: CGFloat = 0, widest: CGFloat = 0
        for subview in subviews {
            let size = subview.sizeThatFits(.unspecified)
            if x > 0, x + size.width > width {
                x = 0
                y += lineHeight + lineSpacing
                lineHeight = 0
            }
            points.append(CGPoint(x: x, y: y))
            x += size.width + spacing
            lineHeight = max(lineHeight, size.height)
            widest = max(widest, x - spacing)
        }
        return (points, CGSize(width: widest, height: y + lineHeight))
    }
}

struct Section<Content: View>: View {
    let heading: String
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(heading.uppercased()).font(.caption.bold()).foregroundStyle(Theme.muted)
            content
        }
    }
}

struct StaleNotice: View {
    let metadata: Metadata

    var body: some View {
        if metadata.state == "stale" {
            Label("Showing saved details from \(shortDate(metadata.fetchedAt)): TMDB isn't responding",
                  systemImage: "clock.arrow.circlepath")
                .font(.callout)
                .foregroundStyle(Theme.movie)
        }
    }
}

struct Poster: View {
    let url: String?
    let width: CGFloat
    let kind: Kind

    var body: some View {
        AsyncImage(url: url.flatMap(URL.init(string:))) { image in
            image.resizable().aspectRatio(contentMode: .fill)
        } placeholder: {
            Image(systemName: kind == .person ? "person.fill" : "film")
                .foregroundStyle(Theme.subtle)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(Theme.line)
        }
        .frame(width: width, height: width * 1.5)
        .clipShape(RoundedRectangle(cornerRadius: width > 60 ? 8 : 4))
    }
}

struct Notice: View {
    let icon: String
    let text: String
    var tint = Theme.subtle

    var body: some View {
        VStack(spacing: 8) {
            Image(systemName: icon).font(.largeTitle).foregroundStyle(tint)
            Text(text).foregroundStyle(Theme.muted).multilineTextAlignment(.center)
        }
        .padding()
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

struct FooterView: View {
    let inDetail: Bool
    let home: Bool
    let rowSelected: Bool
    let canGoBack: Bool
    let lists: Bool

    var body: some View {
        HStack(spacing: 16) {
            Spacer()
            if lists && !inDetail { hint("⌘T ⌘F ⌘W", "To watch, Favorites, Watched") }
            if inDetail {
                if rowSelected {
                    hint("⏎", "Open")
                    hint("⌘⏎", "Open in browser")
                } else {
                    hint("↑↓", "Select")
                    hint("⏎", "Open in browser")
                }
                hint("esc", canGoBack ? "Back" : home ? "Lists" : "Results")
            } else {
                hint("⏎", "Details")
                hint("⌘⏎", "Open in browser")
                hint("esc", "Close")
            }
        }
        .font(.caption)
        .foregroundStyle(Theme.muted)
        .padding(.horizontal, 14)
        .padding(.vertical, 7)
    }

    private func hint(_ key: String, _ label: String) -> some View {
        HStack(spacing: 4) {
            Text(key).bold()
            Text(label)
        }
    }
}

struct Rule: View {
    var body: some View {
        Theme.line.frame(height: 1)
    }
}
