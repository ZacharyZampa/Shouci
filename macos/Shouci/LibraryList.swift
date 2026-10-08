import Foundation
import ShouciCore

// What the library list shows, and in what order. Everything here is a pure
// function of the loaded words: no core calls, no window state.

extension LibraryModel {
    enum Scope: Hashable {
        case all
        case needsReview
        case noCollection
        case recent
        case archived
        case trash
        case collection(String)
        case tag(String)
        /// A saved filter (`SmartCollections.swift`).
        case smart(String)
    }

    enum Sort: String, CaseIterable, Identifiable {
        case added
        case edited
        case pinyin
        case hsk
        case frequency

        var id: Self { self }

        var title: String {
            switch self {
            case .added: "Date Added"
            case .edited: "Date Edited"
            case .pinyin: "Pinyin"
            case .hsk: "HSK Level"
            case .frequency: "Frequency"
            }
        }
    }

    /// What a list row is, for selection.
    enum Pick: Hashable {
        case item(Int64)
        case candidate(String)
    }

    enum Row: Identifiable {
        case item(ItemView)
        case candidate(CandidateView)

        var id: Pick {
            switch self {
            case .item(let item): .item(item.id)
            case .candidate(let candidate): .candidate(candidate.id)
            }
        }
    }

    struct Group: Identifiable {
        let title: String
        let rows: [Row]
        var id: String { title }
    }
}

/// When a word was added, edited, archived, and trashed, parsed once per
/// load so sorting doesn't parse timestamps.
struct ItemDates {
    let added: Date
    let edited: Date
    let archived: Date?
    let trashed: Date?

    init(_ item: ItemView) {
        added = When.parse(item.createdAt) ?? .distantPast
        edited = When.parse(item.modifiedAt) ?? .distantPast
        archived = item.archivedAt.flatMap(When.parse)
        trashed = item.deletedAt.flatMap(When.parse)
    }
}

enum LibraryList {
    typealias Scope = LibraryModel.Scope
    typealias Group = LibraryModel.Group

    /// The words a sidebar scope holds.
    static func items(_ all: [ItemView], in scope: Scope, dates: [Int64: ItemDates], now: Date) -> [ItemView] {
        switch scope {
        case .all:
            return all.filter { $0.lifecycle == .active }
        case .needsReview:
            return all.filter { $0.lifecycle == .active && $0.verification == .needsReview }
        case .noCollection:
            return all.filter { $0.lifecycle == .active && $0.collections.isEmpty }
        case .recent:
            let since = now.addingTimeInterval(-7 * 86_400)
            return all.filter { $0.lifecycle == .active && (dates[$0.id]?.added ?? .distantPast) >= since }
        case .archived:
            return all.filter { $0.lifecycle == .archived }
        case .trash:
            return all.filter { $0.lifecycle == .trashed }
        case .collection(let name):
            return all.filter { $0.lifecycle != .trashed && $0.collections.contains(name) }
        case .tag(let name):
            return all.filter { $0.lifecycle != .trashed && $0.tags.contains(name) }
        case .smart:
            // Its filter decides: the model keeps the words the core matched.
            return all.filter { $0.lifecycle != .trashed }
        }
    }

    /// A scope's words, sorted and grouped under headings: by first letter
    /// of the reading, HSK level, or how common they are, or by when they
    /// were added, edited, archived, or trashed. The Archived and Trash
    /// views always go by when words went there.
    static func groups(
        _ items: [ItemView], scope: Scope, sort: LibraryModel.Sort, dates: [Int64: ItemDates], now: Date
    ) -> [Group] {
        let sort = scope == .trash || scope == .archived ? .added : sort
        // Sort keys are worked out once per word, not once per comparison.
        switch sort {
        case .pinyin:
            let sorted = items.map { (readingOrder($0), $0) }.sorted { $0.0 < $1.0 }.map(\.1)
            return grouped(sorted) { item in
                item.pinyin.first.map { String($0).uppercased() } ?? "No reading"
            }
        case .hsk:
            // Level, then the more common word first, then the reading.
            let sorted = items
                .map { ($0.hskRank ?? .max, $0.frequencyRank ?? .max, readingOrder($0), $0) }
                .sorted { ($0.0, $0.1, $0.2) < ($1.0, $1.1, $1.2) }
                .map(\.3)
            let levels = Dictionary(
                Set(sorted.compactMap(\.hskRank)).map { ($0, hskLevel(rank: $0)) },
                uniquingKeysWith: { first, _ in first })
            return grouped(sorted) { item in item.hskRank.flatMap { levels[$0] } ?? "Not in HSK" }
        case .frequency:
            let sorted = items
                .map { ($0.frequencyRank ?? .max, readingOrder($0), $0) }
                .sorted { ($0.0, $0.1) < ($1.0, $1.1) }
                .map(\.2)
            return grouped(sorted) { bandNames[$0.frequencyBand] ?? "" }
        case .added, .edited:
            break
        }
        let date: (ItemView) -> Date = { item in
            let dated = dates[item.id]
            switch scope {
            case .trash: return dated?.trashed ?? dated?.edited ?? .distantPast
            case .archived: return dated?.archived ?? dated?.edited ?? .distantPast
            default: return (sort == .edited ? dated?.edited : dated?.added) ?? .distantPast
            }
        }
        let sorted = items.map { (date($0), $0) }.sorted { $0.0 > $1.0 }
        // Newest first, so one day's words are neighbours: each day's
        // heading is worked out once, not once per word.
        let calendar = Calendar.current
        var day: DateInterval?
        var heading = ""
        let titled = sorted.map { dated, item in
            if day?.contains(dated) != true {
                day = calendar.dateInterval(of: .day, for: dated)
                heading = period(dated, now: now)
            }
            return (heading, item)
        }
        return grouped(titled)
    }

    /// Readings in alphabetical order, words without one last.
    private static func readingOrder(_ item: ItemView) -> String {
        item.pinyin.isEmpty ? "~" : item.pinyin.lowercased()
    }

    /// The core's bands of the frequency list, most common first.
    static let bands = frequencyBands()
    /// Each band's name, to head the words the core put in it.
    private static let bandNames = Dictionary(uniqueKeysWithValues: bands.map { ($0.band, $0.label) })

    /// Consecutive words with the same heading, as groups.
    private static func grouped(_ items: [ItemView], by title: (ItemView) -> String) -> [Group] {
        grouped(items.map { (title($0), $0) })
    }

    private static func grouped(_ titled: [(String, ItemView)]) -> [Group] {
        var groups: [Group] = []
        var current: (title: String, rows: [LibraryModel.Row])?
        for (key, item) in titled {
            if current?.title != key {
                if let current { groups.append(Group(title: current.title, rows: current.rows)) }
                current = (key, [])
            }
            current?.rows.append(.item(item))
        }
        if let current { groups.append(Group(title: current.title, rows: current.rows)) }
        return groups
    }

    static func period(_ date: Date, now: Date) -> String {
        let calendar = Calendar.current
        if calendar.isDateInToday(date) { return "Today" }
        if calendar.isDateInYesterday(date) { return "Yesterday" }
        if calendar.isDate(date, equalTo: now, toGranularity: .weekOfYear) { return "Earlier this week" }
        if calendar.isDate(date, equalTo: now, toGranularity: .month) { return "Earlier this month" }
        return date.formatted(.dateTime.month(.wide).year())
    }
}
