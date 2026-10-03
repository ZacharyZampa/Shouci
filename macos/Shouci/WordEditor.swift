import ShouciCore
import SwiftUI

/// Edit a saved word, or add one by hand (mockup 04). The dictionary column
/// fills the form; nothing is saved until Save.
struct WordEditor: View {
    let model: LibraryModel
    @State var draft: WordDraft

    @Environment(\.dismiss) private var dismiss
    @State private var entries: [CandidateView] = []
    @State private var dictionary: String?
    @State private var saving = false
    @State private var confirmingDelete = false

    init(model: LibraryModel, draft: WordDraft) {
        self.model = model
        _draft = State(initialValue: draft)
    }

    private var isNew: Bool { draft.original == nil }
    private var preferences: Preferences { model.preferences }

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 0) {
                form
                    .frame(width: 500)
                Divider()
                dictionaryColumn
                    .frame(maxWidth: .infinity)
                    .background(Palette.detail)
            }
            Divider()
            footer
        }
        .frame(width: 980, height: 680)
        .task(id: draft.simplified) {
            try? await Task.sleep(for: .milliseconds(250))
            entries = await model.entries(for: draft.simplified)
        }
    }

    // MARK: Your entry

    private var form: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                CardLabel(isNew ? "New word" : "Your entry")
                HStack(spacing: 12) {
                    labeled("Simplified") {
                        LargeField(text: $draft.simplified, prompt: "")
                    }
                    labeled("Traditional") {
                        LargeField(text: $draft.traditional, prompt: isNew ? "From the dictionary" : "")
                    }
                }
                labeled("Pinyin") {
                    TextField(preferences.pinyinStyle == .numbers ? "xi2 guan4" : "xí guàn", text: $draft.pinyin)
                }
                labeled("Definition") {
                    TextField("", text: $draft.definition, axis: .vertical).lineLimit(3...6)
                }
                labeled("Notes") {
                    TextField("", text: $draft.notes, axis: .vertical).lineLimit(2...5)
                }
                labeled("Tags") {
                    NameList(names: $draft.tags, prompt: "Add tag", style: .accent, known: model.tags.map(\.name))
                }
                labeled("Collections") {
                    NameList(
                        names: $draft.collections, prompt: "Add to collection", style: .plain,
                        known: model.collections.map(\.name))
                }
                Toggle(isOn: $draft.needsReview) {
                    Label("Needs review", systemImage: "exclamationmark.triangle")
                }
                .toggleStyle(.switch)
                .padding(.horizontal, 14)
                .padding(.vertical, 10)
                .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(Palette.warnSoft))
                if isNew {
                    Text("Without a reading and a definition, a new word is saved as needs review.")
                        .font(.system(size: 12))
                        .foregroundStyle(Palette.tertiary)
                }
            }
            .textFieldStyle(.roundedBorder)
            .padding(.horizontal, 28)
            .padding(.vertical, 24)
        }
    }

    private func labeled(_ label: String, @ViewBuilder field: () -> some View) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            Text(label).font(.system(size: 12)).foregroundStyle(Palette.tertiary)
            field()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    // MARK: Dictionary

    private var sources: [String] {
        var seen: [String] = []
        for entry in entries where !seen.contains(entry.dictionary) { seen.append(entry.dictionary) }
        return seen
    }

    private var shown: [CandidateView] {
        let source = dictionary ?? sources.first
        return entries.filter { $0.dictionary == source }
    }

    private var dictionaryColumn: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                HStack {
                    CardLabel("Dictionary")
                    Spacer()
                    if sources.count > 1 {
                        Picker("Dictionary", selection: Binding(get: { dictionary ?? sources.first }, set: { dictionary = $0 })) {
                            ForEach(sources, id: \.self) { id in
                                Text(model.dictionaries.first { $0.id == id }?.name ?? id).tag(Optional(id))
                            }
                        }
                        .pickerStyle(.segmented)
                        .labelsHidden()
                        .fixedSize()
                    }
                }
                if shown.isEmpty {
                    Text(
                        draft.simplified.trimmingCharacters(in: .whitespaces).isEmpty
                            ? "Type the characters to see the dictionary’s entries."
                            : "The dictionary has no entry for \(draft.simplified). Fill in what you know; the word stays marked needs review until it has a reading and a definition."
                    )
                    .font(.system(size: 13))
                    .foregroundStyle(Palette.tertiary)
                }
                ForEach(shown) { entry in
                    reading(entry)
                }
            }
            .padding(.horizontal, 28)
            .padding(.vertical, 24)
        }
    }

    private func reading(_ entry: CandidateView) -> some View {
        let pinyin = preferences.pinyin(raw: entry.pinyin, display: entry.pinyinDisplay)
        let isCurrent = draft.pinyin == pinyin && (draft.traditional.isEmpty || draft.traditional == entry.traditional)
        return VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                Text(entry.simplified).font(.system(size: 30, weight: .medium))
                VStack(alignment: .leading, spacing: 2) {
                    Text(pinyin).font(.system(size: 17))
                    if entry.traditional != entry.simplified {
                        Text("Traditional \(entry.traditional)").font(.system(size: 12)).foregroundStyle(Palette.tertiary)
                    }
                }
                Spacer()
                if !isCurrent {
                    Button("Use this reading") {
                        draft.pinyin = pinyin
                        draft.traditional = entry.traditional
                    }
                    .buttonStyle(SoftButtonStyle())
                }
            }
            VStack(spacing: 0) {
                ForEach(Array(entry.glosses.enumerated()), id: \.offset) { number, gloss in
                    HStack(alignment: .firstTextBaseline, spacing: 10) {
                        Text("\(number + 1)").foregroundStyle(Palette.tertiary).frame(width: 14, alignment: .leading)
                        Text(displayDefinition(definition: gloss)).frame(maxWidth: .infinity, alignment: .leading)
                        Button("Use") { use(entry, definition: gloss, pinyin: pinyin) }
                            .buttonStyle(SoftButtonStyle())
                            .disabled(draft.definition == gloss)
                    }
                    .font(.system(size: 13.5))
                    .padding(.vertical, 8)
                    if number < entry.glosses.count - 1 { Divider() }
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 4)
            .card()
            if entry.glosses.count > 1 {
                Button(entry.glosses.count == 2 ? "Replace my definition with both" : "Replace my definition with all \(entry.glosses.count)") {
                    use(entry, definition: entry.glosses.joined(separator: "; "), pinyin: pinyin)
                }
                .disabled(draft.definition == entry.glosses.joined(separator: "; "))
            }
        }
    }

    /// A meaning from the dictionary; a word with no reading takes the
    /// entry's reading too.
    private func use(_ entry: CandidateView, definition: String, pinyin: String) {
        draft.definition = definition
        if draft.pinyin.trimmingCharacters(in: .whitespaces).isEmpty {
            draft.pinyin = pinyin
            draft.traditional = entry.traditional
        }
    }

    // MARK: Footer

    private var footer: some View {
        HStack(spacing: 10) {
            if let original = draft.original, original.lifecycle != .trashed {
                Button("Delete Word…", role: .destructive) { confirmingDelete = true }
                    .buttonStyle(.borderless)
                    .foregroundStyle(.red)
                    .confirmationDialog("Move \(original.simplified) to the Trash?", isPresented: $confirmingDelete) {
                        Button("Move to Trash", role: .destructive) {
                            model.perform(.trash, on: [original.id])
                            dismiss()
                        }
                    } message: {
                        Text("You can put it back from the Trash.")
                    }
            }
            Spacer()
            Button("Cancel", role: .cancel) { dismiss() }
                .keyboardShortcut(.cancelAction)
            Button(isNew ? "Add Word" : "Save") {
                saving = true
                Task {
                    if await model.save(draft) { dismiss() }
                    saving = false
                }
            }
            .keyboardShortcut(.defaultAction)
            .disabled(saving || draft.simplified.trimmingCharacters(in: .whitespaces).isEmpty)
        }
        .padding(.horizontal, 20)
        .frame(height: 56)
        .background(Palette.window)
    }
}

/// A text field for characters, big enough to read them.
private struct LargeField: View {
    @Binding var text: String
    let prompt: String

    var body: some View {
        TextField(prompt, text: $text)
            .textFieldStyle(.plain)
            .font(.system(size: 22))
            .padding(.horizontal, 10)
            .frame(height: 44)
            .background(RoundedRectangle(cornerRadius: 8, style: .continuous).fill(Palette.field))
            .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).strokeBorder(Palette.line))
    }
}

/// Tags or collections as chips, with a field to add one.
private struct NameList: View {
    @Binding var names: [String]
    let prompt: String
    let style: Chip.Style
    let known: [String]
    @State private var text = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if !names.isEmpty {
                FlowLayout(spacing: 6) {
                    ForEach(names, id: \.self) { name in
                        Chip(text: name, style: style) { names.removeAll { $0 == name } }
                    }
                }
            }
            HStack(spacing: 6) {
                TextField(prompt, text: $text).onSubmit(add)
                let unused = known.filter { !names.contains($0) }
                if !unused.isEmpty {
                    Menu {
                        ForEach(unused, id: \.self) { name in
                            Button(name) { names.append(name) }
                        }
                    } label: {
                        Image(systemName: "chevron.down")
                    }
                    .menuIndicator(.hidden)
                    .fixedSize()
                    .accessibilityLabel("Choose from existing")
                }
            }
        }
    }

    private func add() {
        let name = text.trimmingCharacters(in: .whitespaces)
        guard !name.isEmpty else { return }
        if !names.contains(where: { $0.caseInsensitiveCompare(name) == .orderedSame }) {
            names.append(known.first { $0.caseInsensitiveCompare(name) == .orderedSame } ?? name)
        }
        text = ""
    }
}
