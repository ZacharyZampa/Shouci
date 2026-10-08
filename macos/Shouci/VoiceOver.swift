import AppKit
import ShouciCore

/// Speaks to VoiceOver without moving its cursor. Quick search, the library's
/// search field, and the tag and collection fields keep the keyboard while ↑↓
/// move a highlight somewhere else, so what the highlight lands on, what a
/// search found, and what was saved are said this way. Nothing happens while
/// VoiceOver is off.
@MainActor
enum VoiceOver {
    private static var pending: Task<Void, Never>?

    /// Says `text` now. `interrupt` cuts off what VoiceOver is saying: for
    /// the arrows, where only the row they stop on matters.
    static func say(_ text: String, interrupt: Bool = false) {
        pending?.cancel()
        guard NSWorkspace.shared.isVoiceOverEnabled, !text.isEmpty else { return }
        // The key window: quick search takes the keyboard without making
        // Shouci the active app, and VoiceOver follows the keyboard there.
        let element: Any = NSApp.keyWindow ?? NSApp as Any
        let priority: NSAccessibilityPriorityLevel = interrupt ? .high : .medium
        NSAccessibility.post(
            element: element, notification: .announcementRequested,
            userInfo: [.announcement: text, .priority: priority.rawValue])
    }

    /// Says `text` once nothing newer has come for a moment: what a search
    /// found is said when typing pauses, not after every key.
    static func sayWhenSettled(_ text: String) {
        pending?.cancel()
        pending = Task {
            try? await Task.sleep(for: .milliseconds(400))
            guard !Task.isCancelled else { return }
            say(text)
        }
    }
}

// MARK: - What VoiceOver hears

extension QuickSearch {
    /// A row: the word, its reading and meaning, and whether it is saved.
    func spoken(_ row: Row) -> String {
        switch row {
        case .candidate(let candidate):
            var parts = [
                preferences.headword(simplified: candidate.simplified, traditional: candidate.traditional),
                preferences.pinyin(raw: candidate.pinyin, display: candidate.pinyinDisplay),
                candidate.definitionDisplay,
            ]
            if candidate.isInVocabulary {
                parts.append(candidate.saved?.lifecycle == .archived ? "archived" : "saved")
            } else if candidate.basis == .characterFallback {
                parts.append("same characters")
            } else if candidate.inferred {
                parts.append("partial match")
            } else if candidate.saved?.lifecycle == .trashed {
                parts.append("in the Trash")
            }
            return parts.filter { !$0.isEmpty }.joined(separator: ", ")
        case .saveForReview(let text):
            return "Save “\(text)” as needs review"
        }
    }

    /// What a search found: how many words, then the one ↵ saves.
    var spokenResults: String {
        let rows = self.rows
        let matched = results?.candidates.contains { !$0.inferred } ?? false
        var found: String
        if matched {
            found = rows.count == 1 ? "1 word" : "\(rows.count) words"
        } else {
            let partial = results?.candidates.count ?? 0
            found = "No dictionary match"
            if partial > 0 { found += partial == 1 ? ", 1 partial match" : ", \(partial) partial matches" }
        }
        guard let selection, rows.indices.contains(selection) else { return found }
        return "\(found). \(spoken(rows[selection]))"
    }

    /// Why typing finds nothing yet, while the dictionary isn't searchable.
    var spokenUnsearchable: String {
        switch app.dictionary {
        case .starting: "Opening your library…"
        case .loading(let text): text
        case .failed: "The dictionary isn’t available"
        case .ready: ""
        }
    }
}

extension QuickSearch.Saved {
    /// How the banner and VoiceOver begin: `Saved 习惯`.
    var verb: String {
        switch result.outcome {
        case .inserted: "Saved"
        case .alreadySaved: "Already saved:"
        case .restored: "Restored"
        case .completed: "Completed"
        }
    }

    @MainActor
    func spoken(_ preferences: Preferences) -> String {
        let item = result.item
        let pinyin = preferences.pinyin(raw: item.pinyin, display: item.pinyinDisplay)
        return ["\(verb) \(item.simplified)", pinyin].filter { !$0.isEmpty }.joined(separator: ", ")
    }
}

extension LibraryModel {
    /// A row of the list, as the search field's arrows reach it.
    func spoken(_ pick: Pick) -> String {
        switch pick {
        case .item(let id):
            guard let item = item(id) ?? hits.first(where: { $0.id == id }) else { return "" }
            var parts = [
                preferences.headword(simplified: item.simplified, traditional: item.traditional),
                preferences.pinyin(raw: item.pinyin, display: item.pinyinDisplay),
                item.definitionDisplay,
            ]
            if item.verification == .needsReview { parts.append("needs review") }
            if item.lifecycle == .archived { parts.append("archived") }
            if item.lifecycle == .trashed { parts.append("in the Trash") }
            return parts.filter { !$0.isEmpty }.joined(separator: ", ")
        case .candidate(let id):
            guard let candidate = candidates.first(where: { $0.id == id }) else { return "" }
            return [
                preferences.headword(simplified: candidate.simplified, traditional: candidate.traditional),
                preferences.pinyin(raw: candidate.pinyin, display: candidate.pinyinDisplay),
                candidate.definitionDisplay,
                "from the dictionary",
            ].filter { !$0.isEmpty }.joined(separator: ", ")
        }
    }

    /// What a search found, and the row ↵ acts on.
    var spokenResults: String {
        let added = additions.count
        var found: [String] = []
        if !hits.isEmpty { found.append("\(hits.count) in your vocabulary") }
        if added > 0 { found.append("\(added) from the dictionary") }
        guard !found.isEmpty else { return "No matches" }
        let summary = found.joined(separator: ", ")
        guard selection.count == 1, let pick = selection.first else { return summary }
        return "\(summary). \(spoken(pick))"
    }

    /// The word whose turn it is in the filing sheet.
    var spokenFilingTurn: String {
        guard let filing else { return "" }
        guard let id = filing.current, let word = item(id) else { return "Every word had its turn" }
        let reading = preferences.pinyin(raw: word.pinyin, display: word.pinyinDisplay)
        let headword = preferences.headword(simplified: word.simplified, traditional: word.traditional)
        return "Word \(filing.position + 1) of \(filing.queue.count): "
            + [headword, reading].filter { !$0.isEmpty }.joined(separator: ", ")
    }
}

extension NameSuggestions {
    /// What typing a name found, and the row ↵ picks.
    func spoken(_ kind: GroupKind) -> String {
        let matches = rows.filter { if case .create = $0 { false } else { true } }.count
        let found = switch matches {
        case 0: "No \(kind.noun) matches"
        case 1: "1 \(kind.noun)"
        default: "\(matches) \(kind.noun)s"
        }
        guard let preferred else { return found }
        return "\(found). \(rows[preferred].spoken(kind))"
    }
}

extension NameSuggestions.Row {
    func spoken(_ kind: GroupKind) -> String {
        switch self {
        case .existing(let name, let count):
            "\(name), \(count == 0 ? "empty" : count == 1 ? "1 word" : "\(count) words")"
        case .added(let name):
            "\(name), already added"
        case .create(let name):
            "Create \(kind.noun) “\(name)”"
        }
    }
}
