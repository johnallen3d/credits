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
                    .onTapGesture { model.backToResults() }
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
                       lists: model.listTarget.map { $0.kind != .person } ?? false)
        }
        .frame(width: 720, height: 480)
        .foregroundStyle(Theme.fg)
        .tint(Theme.accent)
        .background(Theme.bg)
        .environment(\.colorScheme, .dark)
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
            if model.query.trimmingCharacters(in: .whitespaces).isEmpty || model.searching {
                Notice(icon: "film", text: "Type to search")
            } else {
                Notice(icon: "questionmark.circle", text: "No matches for “\(model.query)”")
            }
        } else {
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(spacing: 2) {
                        ForEach(model.results) { hit in
                            ResultRow(hit: hit, selected: hit == model.selection, marks: model.marks[hit])
                                .id(hit)
                                .onTapGesture { model.open(hit) }
                        }
                    }
                    .padding(6)
                }
                .onChange(of: model.selection) { _, hit in
                    if let hit { proxy.scrollTo(hit) }
                }
            }
        }
    }
}

struct ResultRow: View {
    let hit: SearchHit
    let selected: Bool
    let marks: TitleMarks?

    var body: some View {
        HStack(spacing: 12) {
            Poster(url: hit.imageUrl, width: 34, kind: hit.kind)
            VStack(alignment: .leading, spacing: 2) {
                Text(hit.displayName).font(.headline).lineLimit(1)
                (Text(hit.kindLabel).foregroundColor(Theme.color(for: hit.kind))
                    + Text(hit.details.isEmpty ? "" : " · " + hit.details).foregroundColor(Theme.muted))
                    .font(.subheadline).lineLimit(1)
            }
            Spacer()
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
        .padding(.horizontal, 10)
        .padding(.vertical, 5)
        .background(selected ? Theme.line : .clear,
                    in: RoundedRectangle(cornerRadius: 8))
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
            ScrollView {
                Group {
                    switch detail {
                    case .title(let t):
                        TitleDetailView(title: t, marks: model.marks[hit] ?? t.marks,
                                        changing: model.changing.contains(hit))
                    case .person(let p): PersonDetailView(person: p)
                    }
                }
                .padding(18)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        } else {
            ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }
}

struct TitleDetailView: View {
    let title: TitleDetail
    let marks: TitleMarks?
    let changing: Bool

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
            ForEach(title.keyCrew, id: \.self) { crew in
                Section(heading: crewHeading(crew.role)) {
                    Text(crew.people.map(\.name).joined(separator: ", "))
                }
            }
            if !title.cast.isEmpty {
                Section(heading: "Cast") {
                    VStack(alignment: .leading, spacing: 3) {
                        ForEach(title.cast.prefix(8), id: \.self) { c in
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
                        ForEach(person.knownForTitles, id: \.self) { c in
                            HStack(spacing: 4) {
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

    private func shortDate(_ iso: String) -> String {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        let date = f.date(from: iso) ?? ISO8601DateFormatter().date(from: iso)
        return date?.formatted(date: .abbreviated, time: .omitted) ?? iso
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
    let lists: Bool

    var body: some View {
        HStack(spacing: 16) {
            Spacer()
            if lists && !inDetail { hint("⌘T ⌘F ⌘W", "To watch, Favorites, Watched") }
            if inDetail {
                hint("⏎", "Open in browser")
                hint("esc", "Back")
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
