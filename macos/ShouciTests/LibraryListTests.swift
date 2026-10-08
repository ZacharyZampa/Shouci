import Foundation
import ShouciCore
import Testing

/// What the library list shows, and under which headings.
@MainActor
struct LibraryListTests {
    private let now = When.parse("2026-01-02T12:00:00.000Z")!

    private func titles(_ items: [ItemView], scope: LibraryModel.Scope = .all, sort: LibraryModel.Sort) -> [String] {
        let dates = Dictionary(uniqueKeysWithValues: items.map { ($0.id, ItemDates($0)) })
        return LibraryList.groups(items, scope: scope, sort: sort, dates: dates, now: now).map(\.title)
    }

    private func ids(_ items: [ItemView], scope: LibraryModel.Scope, sort: LibraryModel.Sort) -> [Int64] {
        let dates = Dictionary(uniqueKeysWithValues: items.map { ($0.id, ItemDates($0)) })
        return LibraryList.groups(items, scope: scope, sort: sort, dates: dates, now: now)
            .flatMap(\.rows).map(\.id).compactMap { pick in
                if case .item(let id) = pick { id } else { nil }
            }
    }

    @Test func byPinyinUnderEachFirstLetterWordsWithoutAReadingLast() {
        let words = [word(1, "学校", pinyin: "xue2 xiao4"), word(2, "米饭"), word(3, "旅行", pinyin: "lu:3 xing2")]
        #expect(titles(words, sort: .pinyin) == ["L", "X", "No reading"])
    }

    @Test func byHSKLevelThenHowCommon() {
        let words = [
            word(1, "猫", hsk: 1, frequency: 900), word(2, "狗", hsk: 1, frequency: 300),
            word(3, "鳄", hsk: 7), word(4, "米饭"),
        ]
        #expect(titles(words, sort: .hsk) == ["HSK 1", "HSK 7–9", "Not in HSK"])
        #expect(ids(words, scope: .all, sort: .hsk) == [2, 1, 3, 4])
    }

    @Test func byFrequencyUnderTheCoresBands() {
        let words = [
            word(1, "的", frequency: 1, band: .top1000), word(2, "米饭"),
            word(3, "旅行", frequency: 2_500, band: .to5000),
        ]
        let names = Dictionary(uniqueKeysWithValues: frequencyBands().map { ($0.band, $0.label) })
        #expect(titles(words, sort: .frequency) == [names[.top1000], names[.to5000], names[.unlisted]].compactMap { $0 })
    }

    @Test func theTrashGoesByWhenWordsWentThereWhateverTheSort() {
        let words = [
            word(1, "安", pinyin: "an1", lifecycle: .trashed, trashed: "2026-01-02T08:00:00.000Z"),
            word(2, "爸", pinyin: "ba4", lifecycle: .trashed, trashed: "2026-01-02T10:00:00.000Z"),
        ]
        #expect(ids(words, scope: .trash, sort: .pinyin) == [2, 1], "newest first, not by reading")
    }

    @Test func eachScopeHoldsItsWords() {
        let words = [
            word(1, "学校", collections: ["Week 1"]), word(2, "米饭"),
            word(3, "旅行", lifecycle: .archived, collections: ["Week 1"]),
            word(4, "猫", lifecycle: .trashed, collections: ["Week 1"]),
        ]
        let held = { (scope: LibraryModel.Scope) in
            LibraryList.items(words, in: scope, dates: [:], now: self.now).map(\.id)
        }
        #expect(held(.noCollection) == [2])
        #expect(held(.collection("Week 1")) == [1, 3], "archived too, never the Trash")
        #expect(held(.archived) == [3])
        #expect(held(.trash) == [4])
    }

    @Test func theSidebarCountsWhatEachViewHolds() {
        var words = [
            word(1, "学校", collections: ["Week 1"]), word(2, "米饭"),
            word(3, "旅行", lifecycle: .archived, collections: ["Week 1"]),
            word(4, "猫", lifecycle: .trashed), word(5, "狗", lifecycle: .archived),
        ]
        words.append(ItemView(
            id: 6, simplified: "饕餮", traditional: "饕餮", pinyin: "", pinyinDisplay: "", definition: "",
            definitionDisplay: "", notes: "", verification: .needsReview, lifecycle: .active,
            source: SourceView(kind: .manual, id: nil, version: nil, importOrigin: nil), tags: [], collections: [],
            destinations: [], frequencyRank: nil, frequencyBand: .unlisted, hskRank: nil,
            createdAt: "2026-01-01T09:00:00.000Z", modifiedAt: "2026-01-01T09:00:00.000Z", archivedAt: nil,
            deletedAt: nil, rev: 1))
        let counts = LibraryList.counts(words)
        for scope in [LibraryModel.Scope.all, .needsReview, .noCollection, .archived, .trash] {
            #expect(counts[scope] == LibraryList.items(words, in: scope, dates: [:], now: now).count, "\(scope)")
        }
    }

    @Test func eachPairedChangeTurnsBackOnceEveryWordHasIt() {
        var review = word(1, "书")
        review.verification = .needsReview
        let active = word(2, "笔")
        let archived = word(3, "猫", lifecycle: .archived)
        let trashed = word(4, "狗", lifecycle: .trashed)

        #expect([review, active].reviewToggle == .setVerification(.needsReview))
        #expect([review].reviewToggle == .setVerification(.confirmed))
        #expect([active, archived].archiveToggle == .archive)
        #expect([archived].archiveToggle == .unarchive)
        #expect([active, trashed].trashToggle == .trash)
        #expect([trashed].trashToggle == .restore)
        // Nothing selected: the menus' titles stay as they start.
        let none: [ItemView] = []
        #expect(none.reviewToggle.title == "Mark as Needs Review")
        #expect(none.archiveToggle.title == "Archive")
        #expect(none.trashToggle.title == "Move to Trash")
        #expect(!none.allTrashed)
    }

    @Test func datesAreReadAgainOnlyForWordsThatChanged() {
        let before = [word(1, "书"), word(2, "笔")]
        let first = ItemDates.of(before, reusing: [:])
        let edited = [word(1, "书"), word(2, "笔", trashed: "2026-03-01T09:00:00.000Z")]
        let second = ItemDates.of(edited, reusing: first)
        #expect(second[1]?.edited == first[1]?.edited)
        #expect(second[2]?.trashed == When.parse("2026-03-01T09:00:00.000Z"))
        #expect(second[2]?.edited == When.parse("2026-03-01T09:00:00.000Z"))
    }
}
