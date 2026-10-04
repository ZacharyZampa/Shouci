import Foundation
import ShouciCore

/// The editor's copy of a word (mockup 04). `original` is nil for a new one.
struct WordDraft: Identifiable {
    let id = UUID()
    var original: ItemView?
    var simplified = ""
    var traditional = ""
    var pinyin = ""
    var definition = ""
    var notes = ""
    var tags: [String] = []
    var collections: [String] = []
    var needsReview = false
    /// The reading as first shown, to tell whether it was edited.
    private(set) var shownPinyin = ""

    init(simplified: String = "") {
        self.simplified = simplified
    }

    init(item: ItemView, style: PinyinStyle) {
        original = item
        simplified = item.simplified
        traditional = item.traditional
        pinyin = style == .numbers ? item.pinyin : item.pinyinDisplay
        shownPinyin = pinyin
        definition = item.definition
        notes = item.notes
        tags = item.tags
        collections = item.collections
        needsReview = item.verification == .needsReview
    }

    var trimmed: WordDraft {
        var copy = self
        copy.simplified = simplified.trimmingCharacters(in: .whitespacesAndNewlines)
        copy.traditional = traditional.trimmingCharacters(in: .whitespacesAndNewlines)
        copy.pinyin = pinyin.trimmingCharacters(in: .whitespacesAndNewlines)
        copy.definition = definition.trimmingCharacters(in: .whitespacesAndNewlines)
        copy.notes = notes.trimmingCharacters(in: .whitespacesAndNewlines)
        return copy
    }

    /// Only the fields that changed; nil when none did.
    func patch(from item: ItemView) -> ItemPatch? {
        let patch = ItemPatch(
            simplified: simplified != item.simplified ? simplified : nil,
            traditional: traditional != item.traditional ? traditional : nil,
            pinyin: pinyin != shownPinyin.trimmingCharacters(in: .whitespacesAndNewlines) ? pinyin : nil,
            definition: definition != item.definition ? definition : nil,
            notes: notes != item.notes ? notes : nil,
            verification: needsReview != (item.verification == .needsReview)
                ? (needsReview ? .needsReview : .confirmed) : nil)
        return patch == ItemPatch() ? nil : patch
    }

    /// Tags and collections gained and lost.
    func groupChanges(from item: ItemView) -> [BulkAction] {
        var actions: [BulkAction] = []
        let addedTags = tags.filter { !item.tags.contains($0) }
        let removedTags = item.tags.filter { !tags.contains($0) }
        if !addedTags.isEmpty { actions.append(.addTags(addedTags)) }
        if !removedTags.isEmpty { actions.append(.removeTags(removedTags)) }
        actions += collections.filter { !item.collections.contains($0) }.map(BulkAction.addToCollection)
        actions += item.collections.filter { !collections.contains($0) }.map(BulkAction.removeFromCollection)
        return actions
    }
}
