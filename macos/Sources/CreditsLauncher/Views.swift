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
                       selectedRow: model.detailRow.flatMap { model.detailRows[safe: $0] },
                       canGoBack: !model.back.isEmpty,
                       lists: model.listTarget?.isTitle ?? false,
                       tags: model.detailTags != nil,
                       editingTags: model.editingTags,
                       image: model.viewing == nil ? model.pageImage != nil : nil)
        }
        .overlay {
            if let image = model.viewing {
                ImageViewer(image: image) { model.viewing = nil }
            }
        }
        .frame(width: 720, height: 480)
        .foregroundStyle(Theme.fg)
        .tint(Theme.accent)
        .background(Theme.bg)
        .clipShape(RoundedRectangle(cornerRadius: 14))
        .onChange(of: model.shown, initial: true) { fieldFocused = true }
        .onChange(of: model.focusSearch) { fieldFocused = true }
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
    @Bindable var model: LauncherModel

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

            if model.tab == .toWatch, let w = model.toWatch, !w.titles.isEmpty {
                ToWatchFilters(model: model)
            }

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
                Notice(icon: model.tab.icon,
                       text: model.tab == .toWatch && !(model.toWatch?.titles.isEmpty ?? true)
                           ? "Nothing on To watch matches these filters" : model.tab.empty)
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

/// To watch's sort and filters, as the web's: each choice counts the titles it
/// would show. The services filter waits for services to be chosen; the tag
/// filter for a tagged title.
struct ToWatchFilters: View {
    @Bindable var model: LauncherModel

    var body: some View {
        let f = model.toWatchFilter
        FlowLayout(spacing: 14, lineSpacing: 6) {
            Picker("Sort", selection: $model.toWatchSort) {
                ForEach(WatchlistSort.allCases, id: \.self) { Text($0.name).tag($0) }
            }
            Picker("Show", selection: $model.toWatchFilter.kind) {
                ForEach([nil, Kind.movie, Kind.tv], id: \.self) { k in
                    Text("\(k == nil ? "All" : k == .movie ? "Movies" : "TV") (\(model.toWatchCount(f.with(\.kind, k))))").tag(k)
                }
            }
            if model.servicesChosen {
                Picker("Available", selection: $model.toWatchFilter.onMyServices) {
                    ForEach([false, true], id: \.self) { mine in
                        Text("\(mine ? "On my services" : "Anywhere") (\(model.toWatchCount(f.with(\.onMyServices, mine))))").tag(mine)
                    }
                }
            }
            Picker("Release", selection: $model.toWatchFilter.release) {
                ForEach([nil, "out_now", "coming_soon"], id: \.self) { r in
                    Text("\(r == nil ? "All" : r == "out_now" ? "Out now" : "Coming soon") (\(model.toWatchCount(f.with(\.release, r))))").tag(r)
                }
            }
            if !model.toWatchTags.isEmpty {
                Picker("Tag", selection: $model.toWatchFilter.tag) {
                    Text("Any (\(model.toWatchCount(f.with(\.tag, nil))))").tag(String?.none)
                    ForEach(model.toWatchTags, id: \.self) { t in
                        Text("#\(t) (\(model.toWatchCount(f.with(\.tag, t))))").tag(String?.some(t))
                    }
                }
            }
        }
        .pickerStyle(.menu)
        .controlSize(.small)
        .frame(maxWidth: .infinity, alignment: .leading)
        .font(.callout)
        .padding(.horizontal, 16)
        .padding(.top, 8)
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
                                            changing: model.changing.contains(hit), credits: credits,
                                            model: model)
                        case .person(let p): PersonDetailView(person: p, credits: credits, model: model)
                        case .season(let s): SeasonDetailView(season: s, credits: credits, model: model)
                        case .credits(let c): CreditsDetailView(titleCredits: c, credits: credits)
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
/// Rows are numbered as in `Detail.rows(kind:)`.
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
    let model: LauncherModel

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
            Backdrop(url: title.backdropUrl)
                .fullSize(model, title.backdropUrl, "\(title.title) backdrop")
            HStack(alignment: .top, spacing: 16) {
                Poster(url: title.posterUrl, width: 120, kind: title.kind)
                    .fullSize(model, title.posterUrl, "\(title.title) poster")
                VStack(alignment: .leading, spacing: 8) {
                    Text(title.title).font(.title.bold())
                    Text(facts).foregroundStyle(Theme.muted)
                    ReleaseDatesView(dates: title.releases?.dates ?? [])
                    HStack(spacing: 6) {
                        ListMarksView(marks: marks, changing: changing)
                        if let url = title.showtimesUrl {
                            Button { model.openLink(url) } label: {
                                Label("Showtimes", systemImage: "ticket")
                                    .font(.callout)
                                    .foregroundStyle(Theme.accent)
                                    .padding(.horizontal, 8).padding(.vertical, 3)
                                    .overlay(Capsule().stroke(Theme.accent))
                                    .contentShape(Capsule())
                            }
                            .buttonStyle(.plain)
                            .help("Look up showtimes in the browser")
                        }
                    }
                    RecommendedByView(model: model, changing: changing)
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
            let crewCount = title.keyCrew.reduce(0) { $0 + $1.people.count }
            let seasons = title.seasonList ?? []
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
            if !seasons.isEmpty {
                Section(heading: "Seasons") {
                    VStack(alignment: .leading, spacing: 3) {
                        ForEach(Array(seasons.enumerated()), id: \.offset) { i, season in
                            credits.row(crewCount + i) {
                                HStack(spacing: 4) {
                                    Text(season.name)
                                    Text(seasonFacts(season)).foregroundStyle(Theme.muted)
                                    if let w = title.watched {
                                        if w.season(season.number) {
                                            Label("Watched", systemImage: "eye.fill").foregroundStyle(Theme.accent)
                                        } else if let pct = w.percent(season.number, of: season.episodeCount) {
                                            Text("· \(pct)% watched").foregroundStyle(Theme.accent)
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            let castShown = Array(title.cast.prefix(Detail.castShown))
            if !castShown.isEmpty || title.hasMoreCredits {
                let first = crewCount + seasons.count
                Section(heading: "Cast") {
                    VStack(alignment: .leading, spacing: 3) {
                        ForEach(Array(castShown.enumerated()), id: \.offset) { i, c in
                            credits.row(first + i) {
                                HStack(spacing: 4) {
                                    Text(c.name)
                                    if let role = c.role, !role.isEmpty {
                                        Text("as \(role)").foregroundStyle(Theme.muted)
                                    }
                                }
                            }
                        }
                        if title.hasMoreCredits {
                            credits.row(first + castShown.count) {
                                HStack(spacing: 4) {
                                    Text("Full cast & crew →").foregroundStyle(Theme.accent)
                                    if let cast = title.castTotal, let crew = title.crewTotal {
                                        Text("(\(cast) cast, \(crew) crew)").foregroundStyle(Theme.muted)
                                    }
                                }
                            }
                            .padding(.top, castShown.isEmpty ? 0 : 3)
                        }
                    }
                }
            }
        }
    }

    private func seasonFacts(_ season: SeasonSummary) -> String {
        [season.year.map { "(\($0))" },
         season.episodeCount.map { "· \($0) episode\($0 == 1 ? "" : "s")" }]
            .compactMap { $0 }.joined(separator: " ")
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

/// A title's whole cast, then its crew by department, as the web's cast and
/// crew page. Rows follow `Detail.rows`: the cast, then each department's people.
struct CreditsDetailView: View {
    let titleCredits: TitleCredits
    let credits: Credits

    /// Each department with the row number of its first person.
    private var departments: [(dept: CrewDepartment, first: Int)] {
        var next = titleCredits.cast.count
        return titleCredits.crew.map { d in
            defer { next += d.people.count }
            return (d, next)
        }
    }

    var body: some View {
        let c = titleCredits
        VStack(alignment: .leading, spacing: 14) {
            VStack(alignment: .leading, spacing: 4) {
                Text("CAST & CREW").font(.caption2.bold()).kerning(0.5).foregroundStyle(Theme.color(for: c.kind))
                (Text(c.title).font(.title.bold())
                    + Text(c.year.map { "  (\($0))" } ?? "").font(.title).foregroundColor(Theme.muted))
                Text("\(c.cast.count) cast · \(c.crew.reduce(0) { $0 + $1.people.count }) crew")
                    .foregroundStyle(Theme.muted)
            }
            StaleNotice(metadata: c.metadata)
            Section(heading: "Cast (\(c.cast.count))") {
                if c.cast.isEmpty {
                    Text("TMDB has no cast listed for this title.").foregroundStyle(Theme.muted)
                } else {
                    people(c.cast, first: 0, prefix: "as ")
                }
            }
            ForEach(departments, id: \.first) { d in
                Section(heading: "\(d.dept.name) (\(d.dept.people.count))") {
                    people(d.dept.people, first: d.first, prefix: "")
                }
            }
            if c.crew.isEmpty {
                Section(heading: "Crew") {
                    Text("TMDB has no crew listed for this title.").foregroundStyle(Theme.muted)
                }
            }
        }
    }

    private func people(_ list: [Credit], first: Int, prefix: String) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            ForEach(Array(list.enumerated()), id: \.offset) { i, p in
                credits.row(first + i) {
                    HStack(spacing: 4) {
                        Text(p.name)
                        let detail = [p.role.flatMap { $0.isEmpty ? nil : prefix + $0 },
                                      p.episodeCount.map { "\($0) episode\($0 == 1 ? "" : "s")" }]
                            .compactMap { $0 }.joined(separator: " · ")
                        if !detail.isEmpty { Text(detail).foregroundStyle(Theme.muted).lineLimit(1) }
                    }
                }
            }
        }
    }
}

/// A movie's release dates by type, as the web's facts list them, upcoming
/// ones in the accent.
struct ReleaseDatesView: View {
    let dates: [ReleaseDate]

    var body: some View {
        if !dates.isEmpty {
            FlowLayout(spacing: 14, lineSpacing: 3) {
                ForEach(dates, id: \.self) { r in
                    HStack(spacing: 5) {
                        Text(r.label).foregroundStyle(Theme.muted)
                        Text(r.display).foregroundStyle(r.upcoming ? Theme.accent : Theme.fg)
                    }
                }
            }
            .font(.callout)
        }
    }
}

/// The tags naming who or what recommended a title, while it's on To watch:
/// each removable, with a field to add more (⌘R) and tags used before to add
/// with a click.
struct RecommendedByView: View {
    @Bindable var model: LauncherModel
    let changing: Bool
    @FocusState private var focused: Bool

    var body: some View {
        if let tags = model.detailTags {
            VStack(alignment: .leading, spacing: 5) {
                FlowLayout(spacing: 6, lineSpacing: 5) {
                    Text("Recommended by").foregroundStyle(Theme.muted)
                    ForEach(tags, id: \.self) { tag in
                        HStack(spacing: 4) {
                            Text("#\(tag)")
                            Button { model.removeTag(tag) } label: {
                                Image(systemName: "xmark").font(.caption2.bold())
                            }
                            .buttonStyle(.plain)
                            .foregroundStyle(Theme.muted)
                            .help("Remove #\(tag)")
                        }
                        .padding(.horizontal, 8).padding(.vertical, 2)
                        .overlay(Capsule().stroke(Theme.line))
                    }
                    HStack(spacing: 4) {
                        TextField(tags.isEmpty ? "#who-or-what" : "#more", text: $model.tagDraft)
                            .textFieldStyle(.plain)
                            .frame(width: 120)
                            .focused($focused)
                            .onSubmit { model.addTypedTags() }
                        Text(focused ? "⏎" : "⌘R").font(.caption).foregroundStyle(Theme.subtle)
                    }
                    .padding(.horizontal, 8).padding(.vertical, 2)
                    .overlay(Capsule().stroke(focused ? Theme.accent : Theme.line))
                }
                if !model.tagSuggestions.isEmpty {
                    FlowLayout(spacing: 6, lineSpacing: 5) {
                        Text("Used before").font(.callout).foregroundStyle(Theme.subtle)
                        ForEach(model.tagSuggestions, id: \.self) { tag in
                            Button("+ #\(tag)") { model.addTag(tag) }
                                .buttonStyle(.plain)
                                .font(.callout)
                                .foregroundStyle(Theme.muted)
                                .help("Add #\(tag)")
                        }
                    }
                }
            }
            .disabled(changing)
            .onChange(of: focused) { model.editingTags = focused }
            .onChange(of: model.focusTags) { focused = true }
            .onDisappear { model.editingTags = false }
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
                    VStack(alignment: .leading, spacing: 6) {
                        ForEach(w.offers, id: \.self) { group in
                            HStack(alignment: .top, spacing: 6) {
                                Text(group.kind.capitalized)
                                    .foregroundStyle(Theme.muted)
                                    .frame(width: 56, height: 30, alignment: .leading)
                                FlowLayout(spacing: 6, lineSpacing: 6) {
                                    ForEach(group.providers, id: \.self) { ProviderChip(provider: $0) }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A provider as on the web: its logo and name, outlined in the accent with a
/// check when it's one of your services. Without a logo, just the name.
struct ProviderChip: View {
    let provider: Provider

    var body: some View {
        HStack(spacing: 6) {
            if let url = provider.logoUrl.flatMap(URL.init(string:)) {
                AsyncImage(url: url) { image in
                    image.resizable().aspectRatio(contentMode: .fill)
                } placeholder: {
                    Theme.line
                }
                .frame(width: 24, height: 24)
                .clipShape(RoundedRectangle(cornerRadius: 5))
            }
            Text(provider.name).lineLimit(1)
            if provider.mine {
                Image(systemName: "checkmark").font(.caption.bold()).foregroundStyle(Theme.accent)
                    .help("One of your services")
            }
        }
        .font(.callout)
        .padding(.leading, provider.logoUrl == nil ? 8 : 3).padding(.trailing, 8)
        .frame(height: 30)
        .background(Theme.card, in: RoundedRectangle(cornerRadius: 7))
        .overlay(RoundedRectangle(cornerRadius: 7)
            .stroke(provider.mine ? Theme.accent : Theme.line, lineWidth: provider.mine ? 2 : 1))
    }
}

/// A season: its facts, the seasons either side, and its episodes, with
/// unwatched episodes' summaries and scores hidden until selected, as the
/// web blurs them until you peek.
struct SeasonDetailView: View {
    let season: SeasonDetail
    let credits: Credits
    let model: LauncherModel

    /// Rows before the episodes: the previous and next seasons.
    private var pager: [SeasonSummary] { [season.previous, season.next].compactMap { $0 } }

    private var facts: String {
        let count = season.episodes.count
        return [season.showTitle,
                season.airDate.map { "First aired \(shortDate($0))" },
                "\(count) episode\(count == 1 ? "" : "s")"]
            .compactMap { $0 }.joined(separator: " · ")
    }

    private var seasonWatched: Bool { season.watched?.season(season.number) ?? false }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(alignment: .top, spacing: 16) {
                Poster(url: season.posterUrl, width: 120, kind: .tv)
                    .fullSize(model, season.posterUrl, "\(season.name) poster")
                VStack(alignment: .leading, spacing: 8) {
                    Text("TV SEASON").font(.caption2.bold()).kerning(0.5).foregroundStyle(Theme.tv)
                    Text(season.name).font(.title.bold())
                    Text(facts).foregroundStyle(Theme.muted)
                    if seasonWatched {
                        Label("Watched", systemImage: "eye.fill").font(.callout).foregroundStyle(Theme.accent)
                    }
                    if let overview = season.overview, !overview.isEmpty {
                        Text(overview).fixedSize(horizontal: false, vertical: true)
                    }
                    if !pager.isEmpty {
                        HStack(spacing: 16) {
                            ForEach(Array(pager.enumerated()), id: \.offset) { i, s in
                                credits.row(i) {
                                    Text(s.number < season.number ? "← \(s.name)" : "\(s.name) →")
                                        .foregroundStyle(Theme.accent)
                                }
                            }
                        }
                    }
                }
            }
            StaleNotice(metadata: season.metadata)
            Section(heading: "Episodes") {
                VStack(alignment: .leading, spacing: 8) {
                    if !seasonWatched && !season.episodes.isEmpty {
                        note(season.watched?.state == "unreadable"
                             ? "Watched marks couldn't be read, so unwatched episodes' summaries and scores are hidden. Select one to peek."
                             : "Summaries and scores of unwatched episodes are hidden. Select one to peek.")
                    }
                    if let ratings = season.episodeRatings.note, !season.episodes.isEmpty {
                        note(ratings)
                    }
                    if season.episodes.isEmpty {
                        Text("TMDB has no episodes listed for this season yet.").foregroundStyle(Theme.muted)
                    }
                    ForEach(Array(season.episodes.enumerated()), id: \.offset) { i, e in
                        credits.row(pager.count + i) {
                            episode(e, peeking: credits.selected == pager.count + i)
                        }
                    }
                }
            }
        }
    }

    private func note(_ text: String) -> some View {
        Text(text).font(.callout).foregroundStyle(Theme.muted).fixedSize(horizontal: false, vertical: true)
    }

    private func episode(_ e: Episode, peeking: Bool) -> some View {
        let watched = season.watched
        let seen = watched?.episode(season.number, e.number) ?? false
        let shown = seen || peeking
        let facts = [e.airDate.map(shortDate) ?? "Air date not announced",
                     e.runtimeMinutes.map { "\($0) min" },
                     (watched?.fromAppleTV(season.number, e.number) ?? false) ? "Watched on Apple TV" : nil]
            .compactMap { $0 }.joined(separator: " · ")
        return HStack(alignment: .top, spacing: 12) {
            Still(url: e.stillUrl)
                .fullSize(model, e.stillUrl, "\(e.title) still")
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    (Text("\(e.number). ").foregroundColor(Theme.muted) + Text(e.title).bold())
                        .lineLimit(1)
                    if seen { Image(systemName: "eye.fill").font(.caption).foregroundStyle(Theme.accent) }
                }
                HStack(spacing: 4) {
                    Text(facts)
                    if let r = e.rating {
                        Text("·")
                        Text(shown ? r.label : "IMDb hidden")
                    }
                }
                .font(.callout)
                .foregroundStyle(Theme.muted)
                if let overview = e.overview, !overview.isEmpty {
                    if shown {
                        Text(overview).font(.callout).lineLimit(4).fixedSize(horizontal: false, vertical: true)
                    } else {
                        Label("Summary hidden: select to peek", systemImage: "eye.slash")
                            .font(.callout).foregroundStyle(Theme.subtle)
                    }
                } else {
                    Text("No summary available.").font(.callout).foregroundStyle(Theme.subtle)
                }
            }
            Spacer(minLength: 0)
        }
        .padding(.vertical, 3)
    }
}

/// A title's backdrop as a banner across the top of its page, as on the web;
/// nothing when it has none or it can't be loaded.
struct Backdrop: View {
    let url: String?

    var body: some View {
        if let url = url.flatMap(URL.init(string:)) {
            AsyncImage(url: url) { phase in
                switch phase {
                case .success(let image):
                    Color.clear
                        .frame(height: 170)
                        .overlay { image.resizable().aspectRatio(contentMode: .fill) }
                        .clipShape(RoundedRectangle(cornerRadius: 8))
                case .failure:
                    EmptyView()
                default:
                    Theme.line.frame(height: 170).clipShape(RoundedRectangle(cornerRadius: 8))
                }
            }
        }
    }
}

/// An episode's still, 16:9.
struct Still: View {
    let url: String?

    var body: some View {
        AsyncImage(url: url.flatMap(URL.init(string:))) { image in
            image.resizable().aspectRatio(contentMode: .fill)
        } placeholder: {
            Image(systemName: "tv").foregroundStyle(Theme.subtle)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(Theme.line)
        }
        .frame(width: 112, height: 63)
        .clipShape(RoundedRectangle(cornerRadius: 4))
    }
}

struct PersonDetailView: View {
    let person: PersonDetail
    let credits: Credits
    @Bindable var model: LauncherModel

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
                    .fullSize(model, person.profileUrl, person.name)
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
            FilmographyView(person: person, credits: credits, model: model)
        }
    }
}

/// A person's full filmography by department, as the web's: sorted by date or
/// rating, shown as All, Movies or TV with counts, and each credit with To watch
/// and Favorite beside it. Rows follow the known-for titles in `Detail.rows`.
struct FilmographyView: View {
    let person: PersonDetail
    let credits: Credits
    @Bindable var model: LauncherModel

    /// Each department shown with the row number of its first credit.
    private var groups: [(group: FilmographyGroup, first: Int)] {
        var next = person.knownForTitles.count
        return person.groups(model.personKind).map { g in
            defer { next += g.entries.count }
            return (g, next)
        }
    }

    private var sort: Binding<FilmographySort> {
        Binding(get: { FilmographySort(rawValue: person.sort) ?? .date }, set: { model.sortPerson($0) })
    }

    /// Why scores are missing, or where unscored credits go, worded as on the web.
    private var ratingNote: String? {
        guard let r = person.sortRatings else { return nil }
        switch r.unavailableReason {
        case "not_configured": return "Ratings aren't configured on this server, so credits stay in date order."
        case .some: return "Some ratings couldn't be loaded right now."
        case nil:
            if let since = r.staleSince { return "Ratings saved from \(shortDate(since)): OMDb isn't responding." }
            if person.sort == "rating", let n = r.unrated, n > 0 {
                return "Credits without a Rotten Tomatoes or IMDb score are listed last, newest first."
            }
            return nil
        }
    }

    private var listsNote: String? {
        let unread = model.personListsUnread
        guard !unread.isEmpty else { return nil }
        let names = unread.map(\.name).joined(separator: " and ")
        return "\(names) couldn't be read, so \(unread.count == 1 ? "its" : "their") marks are unknown here. Changing one checks the title first."
    }

    var body: some View {
        Section(heading: "Filmography") {
            if person.filmography.isEmpty {
                Text("TMDB has no credits listed for this person.").foregroundStyle(Theme.muted)
            } else {
                VStack(alignment: .leading, spacing: 10) {
                    HStack(spacing: 14) {
                        Picker("Sort", selection: sort) {
                            ForEach(FilmographySort.allCases, id: \.self) { Text($0.name).tag($0) }
                        }
                        .fixedSize()
                        Picker("Show", selection: $model.personKind) {
                            ForEach([nil, Kind.movie, Kind.tv], id: \.self) { k in
                                Text("\(k == nil ? "All" : k == .movie ? "Movies" : "TV") (\(person.count(k)))").tag(k)
                            }
                        }
                        .fixedSize()
                        if model.personSorting { ProgressView().controlSize(.small) }
                    }
                    .pickerStyle(.menu)
                    .controlSize(.small)
                    .font(.callout)
                    if let error = model.personSortError {
                        note("Couldn't load that order (\(error)); showing the last one.", tint: Theme.love)
                    } else if let ratingNote {
                        note(ratingNote, tint: Theme.muted)
                    }
                    if let listsNote { note(listsNote, tint: Theme.love) }
                    if groups.isEmpty {
                        Text("No \(model.personKind == .tv ? "TV" : "movie") credits listed for this person.")
                            .foregroundStyle(Theme.muted)
                    }
                    ForEach(groups, id: \.group.department) { g in
                        VStack(alignment: .leading, spacing: 3) {
                            Text("\(g.group.department) (\(g.group.entries.count))")
                                .font(.callout.bold()).foregroundStyle(Theme.muted)
                            ForEach(Array(g.group.entries.enumerated()), id: \.offset) { i, c in
                                credits.row(g.first + i) {
                                    row(c, acting: g.group.isActing)
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    private func note(_ text: String, tint: Color) -> some View {
        Text(text).font(.callout).foregroundStyle(tint).fixedSize(horizontal: false, vertical: true)
    }

    private func row(_ c: PersonCredit, acting: Bool) -> some View {
        let hit = c.hit
        let marks = model.marks[hit]
        let changing = model.changing.contains(hit)
        let role = c.role.flatMap { $0.isEmpty ? nil : acting ? "as \($0)" : $0 }
        let episodes = c.episodeCount.map { "\($0) episode\($0 == 1 ? "" : "s")" }
        return HStack(spacing: 6) {
            Text(c.year ?? "TBA").foregroundStyle(Theme.muted).frame(width: 38, alignment: .leading)
            Image(systemName: c.kind == .tv ? "tv" : "film")
                .font(.caption)
                .foregroundStyle(Theme.color(for: c.kind))
            Text(c.title).lineLimit(1).layoutPriority(1)
            if let detail = [role, episodes].compactMap({ $0 }).joined(separator: " · ").nilIfEmpty {
                Text("· \(detail)").foregroundStyle(Theme.muted).lineLimit(1)
            }
            Spacer(minLength: 8)
            if let r = c.rating { Text(r.label).font(.callout).foregroundStyle(Theme.muted) }
            if changing { ProgressView().controlSize(.mini) }
            ForEach([ListAction.toWatch, .favorite], id: \.self) { list in
                let on = marks?[list]
                Button { model.toggle(list, hit: hit) } label: {
                    Image(systemName: on == true ? list.icon + ".fill" : list.icon)
                        .foregroundStyle(on == true ? Theme.accent : on == nil ? Theme.subtle : Theme.muted)
                        .frame(width: 18)
                }
                .buttonStyle(.plain)
                .disabled(changing)
                .help(on == nil ? "\(list.name): couldn't read; click to check and change"
                                : "\(on! ? "Remove from" : "Add to") \(list.name)")
            }
        }
    }
}

extension String {
    var nilIfEmpty: String? { isEmpty ? nil : self }
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

extension View {
    /// Opens the image full size on a click, when there is one.
    @ViewBuilder func fullSize(_ model: LauncherModel, _ url: String?, _ label: String) -> some View {
        if url != nil {
            onTapGesture { model.view(url, label) }
                .help("View full size (⌘Y for the poster or photo)")
        } else {
            self
        }
    }
}

/// An image full size over the panel, as the web's lightbox: the copy already
/// shown fills in until the original loads, or if it can't. Pinch to zoom and
/// drag to pan; a click, esc or ⌘Y closes it.
struct ImageViewer: View {
    let image: LauncherModel.FullImage
    let close: () -> Void
    @State private var scale: CGFloat = 1
    @State private var settledScale: CGFloat = 1
    @State private var offset: CGSize = .zero
    @State private var settledOffset: CGSize = .zero

    var body: some View {
        ZStack {
            Color.black.opacity(0.88)
            AsyncImage(url: image.full) { phase in
                switch phase {
                case .success(let full):
                    fitted(full)
                case .failure:
                    shown.overlay(alignment: .bottom) {
                        Label("Couldn't load the full-size image; showing the smaller copy", systemImage: "exclamationmark.triangle")
                            .font(.callout).foregroundStyle(.white)
                            .padding(.horizontal, 10).padding(.vertical, 5)
                            .background(.black.opacity(0.7), in: Capsule())
                            .padding(.bottom, 12)
                    }
                default:
                    shown.overlay { ProgressView().controlSize(.small).tint(.white) }
                }
            }
            .scaleEffect(scale)
            .offset(offset)
            .padding(16)
        }
        .contentShape(Rectangle())
        .onTapGesture(perform: close)
        .gesture(MagnifyGesture()
            .onChanged { scale = max(1, min(6, settledScale * $0.magnification)) }
            .onEnded { _ in
                settledScale = scale
                if scale == 1 { offset = .zero; settledOffset = .zero }
            })
        .simultaneousGesture(DragGesture(minimumDistance: 4)
            .onChanged { d in
                guard scale > 1 else { return }
                offset = CGSize(width: settledOffset.width + d.translation.width,
                                height: settledOffset.height + d.translation.height)
            }
            .onEnded { _ in settledOffset = offset })
        .overlay(alignment: .topTrailing) {
            Button(action: close) {
                Image(systemName: "xmark").font(.title3.bold()).foregroundStyle(.white)
                    .frame(width: 34, height: 34)
                    .background(.black.opacity(0.6), in: Circle())
            }
            .buttonStyle(.plain)
            .help("Close (esc)")
            .padding(12)
        }
        .accessibilityLabel(image.label)
        .transition(.opacity)
    }

    private var shown: some View {
        AsyncImage(url: image.shown) { small in fitted(small) } placeholder: { Color.clear }
    }

    private func fitted(_ image: Image) -> some View {
        image.resizable().aspectRatio(contentMode: .fit)
            .clipShape(RoundedRectangle(cornerRadius: 4))
            .frame(maxWidth: .infinity, maxHeight: .infinity)
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
    let selectedRow: SearchHit?
    let canGoBack: Bool
    let lists: Bool
    var tags = false
    var editingTags = false
    /// Whether the page has an image ⌘Y opens; nil while one is open.
    var image: Bool? = false

    var body: some View {
        HStack(spacing: 16) {
            Spacer()
            if image == nil {
                hint("esc", "Close")
            } else {
                hints
            }
        }
        .font(.caption)
        .foregroundStyle(Theme.muted)
        .padding(.horizontal, 14)
        .padding(.vertical, 7)
    }

    @ViewBuilder private var hints: some View {
        if inDetail && image == true && !editingTags { hint("⌘Y", "Full size") }
        if lists && !inDetail { hint("⌘T ⌘F ⌘W", "To watch, Favorites, Watched") }
        if editingTags {
            hint("⏎", "Add tags")
            hint("esc", "Done")
        } else if inDetail {
            if tags { hint("⌘R", "Tag") }
            if selectedRow?.kind == .episode {
                hint("⌘W", "Watched")
                hint("⏎", "Open in browser")
            } else if selectedRow != nil {
                if selectedRow!.isTitle { hint("⌘T ⌘F", "To watch, Favorites") }
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
