import Foundation
import ShouciCore

// What the library list shows, and in what order. Everything here is a pure
// function of the loaded words: no core calls, no window state.

extension LibraryModel {
    enum Scope: Hashable {
        case all
        case needsReview
        case recent
        case archived
        case trash
        case collection(String)
        case tag(String)
    }

    enum Sort: String, CaseIterable, Identifiable {
        case added
        case edited
        case pinyin

        var id: Self { self }

        var title: String {
            switch self {
            case .added: "Date Added"
            case .edited: "Date Edited"
            case .pinyin: "Pinyin"
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
        }
    }

    /// A scope's words, sorted and grouped under headings: by first letter
    /// of the reading, or by when they were added, edited, archived, or
    /// trashed.
    static func groups(
        _ items: [ItemView], scope: Scope, sort: LibraryModel.Sort, dates: [Int64: ItemDates], now: Date
    ) -> [Group] {
        if sort == .pinyin {
            let sorted = items.sorted { ($0.pinyin.isEmpty ? "~" : $0.pinyin.lowercased()) < ($1.pinyin.isEmpty ? "~" : $1.pinyin.lowercased()) }
            return grouped(sorted) { item in
                item.pinyin.first.map { String($0).uppercased() } ?? "No reading"
            }
        }
        let date: (ItemView) -> Date = { item in
            let dated = dates[item.id]
            switch scope {
            case .trash: return dated?.trashed ?? dated?.edited ?? .distantPast
            case .archived: return dated?.archived ?? dated?.edited ?? .distantPast
            default: return (sort == .edited ? dated?.edited : dated?.added) ?? .distantPast
            }
        }
        return grouped(items.sorted { date($0) > date($1) }) { period(date($0), now: now) }
    }

    /// Consecutive words with the same heading, as groups.
    private static func grouped(_ items: [ItemView], by title: (ItemView) -> String) -> [Group] {
        var groups: [Group] = []
        var current: (title: String, rows: [LibraryModel.Row])?
        for item in items {
            let key = title(item)
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
