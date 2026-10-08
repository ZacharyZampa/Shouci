import ShouciCore
import SwiftUI

// Filing words into collections, one after another (⇧⌘C, or File Words in
// Not in a Collection). ↵ files the word shown and shows the next; the
// collection used last is picked before anything is typed, so a run of words
// for one collection is ↵, ↵, ↵. ⇥ skips a word, esc stops. Several selected
// words are filed together, at once. Each filing is its own change, so ⌘Z
// takes back the last one.

extension LibraryModel {
    /// Files the selected words together, or each word in turn from the
    /// selected one (or the top of the list) down.
    func startFiling() {
        let picked = selectedItems.filter { $0.lifecycle != .trashed }
        if picked.count > 1 {
            filing = Filing(queue: picked.map(\.id), together: true)
            return
        }
        // The list as it is now: words filed from Not in a Collection leave
        // it, and the turn order must not shift under them.
        let listed = order.compactMap { pick -> Int64? in
            guard case .item(let id) = pick, item(id)?.lifecycle != .trashed else { return nil }
            return id
        }
        let start = picked.first.flatMap { listed.firstIndex(of: $0.id) } ?? 0
        guard start < listed.count else { return }
        filing = Filing(queue: Array(listed[start...]), together: false)
        selection = [.item(listed[start])]
    }

    /// The words being filed now: the one whose turn it is, or every word
    /// filed together.
    var filingWords: [ItemView] {
        guard let filing else { return [] }
        let ids = filing.together ? filing.queue : filing.current.map { [$0] } ?? []
        return ids.compactMap(item)
    }

    func file(into collection: String) {
        let ids = filingWords.map(\.id)
        guard !ids.isEmpty else { return }
        recentCollections.removeAll { $0.caseInsensitiveCompare(collection) == .orderedSame }
        recentCollections.insert(collection, at: 0)
        let filed = record("Add to Collection", ids: ids) { _ = try $0.bulk(ids: ids, action: .addToCollection(collection)) }
        // The next word shows at once, so ↵, ↵, ↵ never files one twice;
        // words are counted once filed. A failed filing says why.
        let session = filing?.id
        Task { if await filed.value, filing?.id == session { filing?.filed += ids.count } }
        if filing?.together == true {
            filing = nil
        } else {
            nextFilingTurn()
        }
    }

    /// Leaves the word shown where it is and shows the next.
    func skipFiling() {
        nextFilingTurn()
    }

    private func nextFilingTurn() {
        guard var filing else { return }
        filing.position += 1
        // Words trashed in the meantime are passed over.
        while let id = filing.current, item(id).map({ $0.lifecycle == .trashed }) ?? true {
            filing.position += 1
        }
        self.filing = filing
        if let id = filing.current { selection = [.item(id)] }
    }
}

/// The filing sheet: the word, a collection field, and what the keys do.
struct FilingSheet: View {
    @Bindable var model: LibraryModel

    var body: some View {
        if let filing = model.filing {
            VStack(alignment: .leading, spacing: 16) {
                VStack(alignment: .leading, spacing: 3) {
                    Text("File in a Collection").font(.system(size: 17, weight: .semibold))
                    Text(progress(filing)).foregroundStyle(Palette.secondary)
                }
                let words = model.filingWords
                if words.isEmpty {
                    finished(filing)
                } else {
                    card(words)
                    if let problem = model.problem { Notice(text: problem) }
                    NameInput(
                        kind: .collection, known: model.collections, present: shared(words),
                        placement: .list, focusOnAppear: true, recent: model.recentCollections,
                        choose: { model.file(into: $0) },
                        dismiss: { model.filing = nil },
                        tab: filing.together ? nil : { model.skipFiling() })
                    HStack(spacing: 14) {
                        KeyHint(key: "↵", action: "File")
                        if !filing.together { KeyHint(key: "⇥", action: "Skip") }
                        Spacer()
                        KeyHint(key: "esc", action: "Done")
                    }
                    .font(.system(size: 12))
                    .foregroundStyle(Palette.tertiary)
                }
            }
            .padding(22)
            .frame(width: 440)
            .onDisappear { model.problem = nil }
        }
    }

    private func progress(_ filing: LibraryModel.Filing) -> String {
        if filing.together { return "\(filing.queue.count) words, filed together" }
        if filing.current == nil { return filing.filed == 1 ? "1 word filed" : "\(filing.filed) words filed" }
        return "Word \(filing.position + 1) of \(filing.queue.count)"
    }

    /// The word whose turn it is, or the words filed together.
    @ViewBuilder
    private func card(_ words: [ItemView]) -> some View {
        if words.count == 1, let word = words.first {
            VStack(alignment: .leading, spacing: 4) {
                HStack(alignment: .firstTextBaseline, spacing: 10) {
                    Text(model.preferences.headword(simplified: word.simplified, traditional: word.traditional))
                        .font(.system(size: 28, weight: .medium))
                    Text(model.preferences.pinyin(raw: word.pinyin, display: word.pinyinDisplay))
                        .font(.system(size: 15))
                        .foregroundStyle(Palette.secondary)
                    Spacer()
                    if let level = word.hskRank {
                        Chip(text: hskLevel(rank: level), style: .plain)
                    }
                }
                Text(word.definitionDisplay.isEmpty ? "No definition" : word.definitionDisplay)
                    .foregroundStyle(word.definitionDisplay.isEmpty ? Palette.tertiary : Palette.text)
                    .lineLimit(2)
                if !word.collections.isEmpty {
                    FlowLayout(spacing: 6) {
                        ForEach(word.collections, id: \.self) { Chip(text: $0, style: GroupKind.collection.chipStyle) }
                    }
                    .padding(.top, 4)
                }
            }
            .padding(14)
            .frame(maxWidth: .infinity, alignment: .leading)
            .card()
        } else {
            FlowLayout(spacing: 6) {
                ForEach(words.prefix(40)) { Chip(text: $0.simplified, style: .plain) }
                if words.count > 40 { Chip(text: "+\(words.count - 40) more", style: .plain) }
            }
        }
    }

    private func finished(_ filing: LibraryModel.Filing) -> some View {
        HStack {
            Text("Every word had its turn.")
                .foregroundStyle(Palette.secondary)
            Spacer()
            Button("Done") { model.filing = nil }
                .keyboardShortcut(.defaultAction)
        }
    }

    /// Collections every word has, so the field shows them as added.
    private func shared(_ words: [ItemView]) -> [String] {
        guard let first = words.first else { return [] }
        return first.collections.filter { name in words.allSatisfy { $0.collections.contains(name) } }
    }
}
