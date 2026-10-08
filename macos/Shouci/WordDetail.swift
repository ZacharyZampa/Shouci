import ShouciCore
import SwiftUI

/// The right column: one saved word, a dictionary word to add, or what to
/// do with several selected words.
struct WordDetail: View {
    let model: LibraryModel

    var body: some View {
        Group {
            if model.selectedItems.count > 1 {
                BulkPanel(model: model, items: model.selectedItems)
            } else if let item = model.selectedItem {
                ItemDetail(model: model, item: item)
            } else if let candidate = model.selectedCandidate {
                CandidatePreview(model: model, candidate: candidate)
            } else {
                ContentUnavailableView("Select a word", systemImage: "character.book.closed")
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Palette.detail)
    }
}

/// Big characters, reading, and traditional form.
private struct Headword: View {
    let simplified: String
    let traditional: String
    let reading: String

    var body: some View {
        HStack(alignment: .top, spacing: 20) {
            Text(simplified)
                .font(.system(size: 60, weight: .medium))
                .textSelection(.enabled)
                .lineLimit(1)
                .minimumScaleFactor(0.5)
            VStack(alignment: .leading, spacing: 6) {
                Text(reading.isEmpty ? "No reading yet" : reading)
                    .font(.system(size: 24))
                    .foregroundStyle(reading.isEmpty ? Palette.tertiary : Palette.text)
                    .textSelection(.enabled)
                if !traditional.isEmpty && traditional != simplified {
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        Text("Traditional").font(.system(size: 12.5)).foregroundStyle(Palette.tertiary)
                        Text(traditional).font(.system(size: 18)).foregroundStyle(Palette.secondary)
                            .textSelection(.enabled)
                    }
                }
            }
            .padding(.top, 4)
        }
    }
}

/// Numbered senses in a card.
private struct Senses: View {
    let glosses: [String]

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(glosses.enumerated()), id: \.offset) { number, gloss in
                HStack(alignment: .firstTextBaseline, spacing: 12) {
                    Text("\(number + 1)").foregroundStyle(Palette.tertiary).frame(width: 16, alignment: .leading)
                    Text(displayDefinition(definition: gloss)).textSelection(.enabled)
                }
                .font(.system(size: 13.5))
                .padding(.vertical, 7)
            }
        }
    }
}

// MARK: - A saved word

private struct ItemDetail: View {
    let model: LibraryModel
    let item: ItemView

    @State private var dictionary: String?
    @State private var entries: [DictionaryEntryView] = []
    @State private var lookupProblem: String?

    private var preferences: Preferences { model.preferences }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                HStack(alignment: .top) {
                    Headword(
                        simplified: item.simplified, traditional: item.traditional,
                        reading: preferences.pinyin(raw: item.pinyin, display: item.pinyinDisplay))
                    Spacer()
                    if item.lifecycle != .trashed {
                        Button("Edit") { model.edit(item) }
                    }
                    Menu {
                        WordActions(model: model, items: [item])
                    } label: {
                        Image(systemName: "ellipsis")
                    }
                    .menuIndicator(.hidden)
                    .fixedSize()
                    .accessibilityLabel("More actions")
                }
                status
                yourEntry
                    .zIndex(1)
                dictionarySection
                provenance
            }
            .padding(.horizontal, 32)
            .padding(.vertical, 26)
            .frame(maxWidth: 760, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .leading)
            .endsEditingOnClick()
        }
        .task(id: LookupKey(item: item, dictionary: chosenDictionary)) {
            await lookUp()
        }
    }

    private var status: some View {
        FlowLayout(spacing: 8) {
            if item.verification == .needsReview {
                Chip(text: "Needs review", icon: "exclamationmark.triangle", style: .warn)
            } else {
                Chip(text: "Confirmed", icon: "checkmark", style: .ok)
            }
            if let level = item.hskRank { Chip(text: hskLevel(rank: level), style: .plain) }
            if let rank = item.frequencyRank {
                Chip(text: "Frequency #\(rank.formatted())", style: .plain)
                    .help("Its place in the dictionary’s list of the most common words")
            }
            if item.lifecycle == .archived { Chip(text: "Archived", icon: "archivebox", style: .plain) }
            if item.lifecycle == .trashed { Chip(text: "In the Trash", icon: "trash", style: .plain) }
            if !item.destinations.isEmpty {
                Chip(text: "In \(item.destinations.map(connectorName).joined(separator: ", "))", style: .plain)
            }
        }
    }

    /// Notes, tags, and collections are edited right here; the definition
    /// and reading go through the editor, beside the dictionary.
    private var yourEntry: some View {
        VStack(alignment: .leading, spacing: 8) {
            CardLabel("Your entry")
            VStack(spacing: 0) {
                field("Definition") {
                    Text(item.definitionDisplay.isEmpty ? "None yet" : item.definitionDisplay)
                        .foregroundStyle(item.definitionDisplay.isEmpty ? Palette.tertiary : Palette.text)
                }
                Divider()
                field("Notes") {
                    if isEditable {
                        let id = item.id
                        NotesField(
                            notes: item.notes, editorOpen: model.editor != nil,
                            typing: { text in model.typedNotes = text.map { (id, $0) } },
                            save: { notes in model.update(id, patch: ItemPatch(notes: notes)) })
                    } else {
                        Text(item.notes.isEmpty ? "None" : item.notes)
                            .foregroundStyle(item.notes.isEmpty ? Palette.tertiary : Palette.text)
                    }
                }
                Divider()
                // Each name list's menu opens over the rows below it.
                field("Tags") { nameList(.tag, item.tags, known: model.tags) }
                    .zIndex(2)
                Divider()
                field("Collections") { nameList(.collection, item.collections, known: model.collections) }
                    .zIndex(1)
            }
            .endsEditingOnClick()
            .card()
            // Half-typed notes or a name stay with their word.
            .id(item.id)
        }
    }

    /// A word in the Trash is only shown.
    private var isEditable: Bool { item.lifecycle != .trashed }

    @ViewBuilder
    private func nameList(_ kind: GroupKind, _ names: [String], known: [GroupView]) -> some View {
        if isEditable {
            let id = item.id
            NameList(
                kind: kind, names: names, known: known,
                add: { name in
                    model.update(id, actions: [kind == .tag ? .addTags([name]) : .addToCollection(name)])
                },
                remove: { name in
                    model.update(id, actions: [kind == .tag ? .removeTags([name]) : .removeFromCollection(name)])
                })
        } else if names.isEmpty {
            Text("None").foregroundStyle(Palette.tertiary)
        } else {
            FlowLayout { ForEach(names, id: \.self) { Chip(text: $0, style: kind.chipStyle) } }
        }
    }

    private func field(_ label: String, @ViewBuilder content: () -> some View) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 16) {
            Text(label).foregroundStyle(Palette.tertiary).frame(width: 84, alignment: .leading)
            content()
                .textSelection(.enabled)
                .lineSpacing(3)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .font(.system(size: 13.5))
        .padding(.horizontal, 16)
        .padding(.vertical, 11)
    }

    // MARK: Dictionary

    private var installed: [DictionaryView] { model.dictionaries }

    private var chosenDictionary: String? {
        dictionary ?? installed.first(where: \.enabled)?.id ?? installed.first?.id
    }

    private var sameWord: DictionaryEntryView? { entries.first(where: \.sameWord) }

    private var dictionarySection: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                CardLabel("Dictionary")
                Spacer()
                if installed.count > 1 {
                    Picker("Dictionary", selection: Binding(get: { chosenDictionary }, set: { dictionary = $0 })) {
                        ForEach(installed) { Text($0.name).tag(Optional($0.id)) }
                    }
                    .pickerStyle(.segmented)
                    .labelsHidden()
                    .fixedSize()
                }
            }
            VStack(alignment: .leading, spacing: 0) {
                if let lookupProblem {
                    Text(lookupProblem).foregroundStyle(Palette.tertiary).padding(.vertical, 8)
                } else if let entry = sameWord {
                    Senses(glosses: entry.glosses)
                    Divider().padding(.top, 4)
                    HStack {
                        Spacer()
                        Button("Replace my definition") {
                            if let chosenDictionary { model.useDefinition(of: item, from: chosenDictionary) }
                        }
                        .buttonStyle(SoftButtonStyle())
                        .disabled(entry.glosses.joined(separator: "; ") == item.definition || item.lifecycle == .trashed)
                    }
                    .padding(.top, 9)
                    .padding(.bottom, 5)
                } else if entries.isEmpty {
                    Text("\(dictionaryName(chosenDictionary)) has no entry for \(item.simplified).")
                        .foregroundStyle(Palette.tertiary)
                        .padding(.vertical, 8)
                } else {
                    Text("\(dictionaryName(chosenDictionary)) reads \(item.simplified) differently:")
                        .foregroundStyle(Palette.tertiary)
                        .padding(.vertical, 8)
                    ForEach(entries) { entry in
                        VStack(alignment: .leading, spacing: 2) {
                            Text(preferences.pinyin(raw: entry.pinyin, display: entry.pinyinDisplay))
                                .font(.system(size: 13.5, weight: .semibold))
                            Text(entry.definitionDisplay).foregroundStyle(Palette.secondary)
                        }
                        .padding(.vertical, 5)
                    }
                    Divider().padding(.top, 4)
                    HStack {
                        Spacer()
                        Button("Choose a Reading…") { model.edit(item) }.buttonStyle(SoftButtonStyle())
                    }
                    .padding(.top, 9)
                    .padding(.bottom, 5)
                }
            }
            .font(.system(size: 13.5))
            .padding(.horizontal, 16)
            .padding(.vertical, 6)
            .frame(maxWidth: .infinity, alignment: .leading)
            .card()
        }
    }

    private func lookUp() async {
        guard let chosenDictionary else {
            entries = []
            lookupProblem = "No dictionary is installed yet."
            return
        }
        do {
            entries = try await model.lookup(item, in: chosenDictionary)
            lookupProblem = nil
        } catch {
            entries = []
            lookupProblem = describe(error)
        }
    }

    // MARK: Provenance

    private var provenance: some View {
        Text(provenanceText)
            .font(.system(size: 12))
            .foregroundStyle(Palette.tertiary)
    }

    private var provenanceText: String {
        var parts: [String] = []
        let created = When.parse(item.createdAt)
        let on = created.map { " · \($0.formatted(date: .abbreviated, time: .omitted))" } ?? ""
        switch item.source.kind {
        case .dictionary: parts.append("Saved from \(dictionaryName(item.source.id))\(on)")
        case .import: parts.append("Imported from \(connectorName(item.source.id ?? "a file"))\(on)")
        case .manual: parts.append("Added by hand\(on)")
        }
        if let created, let edited = When.parse(item.modifiedAt), edited.timeIntervalSince(created) > 1 {
            parts.append("Edited \(edited.formatted(date: .abbreviated, time: .omitted))")
        }
        return parts.joined(separator: " · ")
    }

    private func dictionaryName(_ id: String?) -> String {
        guard let id else { return "The dictionary" }
        return installed.first { $0.id == id }?.name ?? id
    }

    private func connectorName(_ id: String) -> String {
        switch id {
        case "pleco": "Pleco"
        case "anki": "Anki"
        default: id
        }
    }

    /// What the lookup reads: notes, tags, and collections changing in place
    /// don't look the word up again.
    private struct LookupKey: Hashable {
        let id: Int64
        let simplified: String
        let traditional: String
        let pinyin: String
        let dictionary: String?

        init(item: ItemView, dictionary: String?) {
            id = item.id
            simplified = item.simplified
            traditional = item.traditional
            pinyin = item.pinyin
            self.dictionary = dictionary
        }
    }
}

/// Notes, edited where they're shown: saved on ↵ or when the field lets go of
/// the keyboard, put back on Esc. ⌥↵ starts a new line.
private struct NotesField: View {
    let notes: String
    /// The editor sheet is open: the note being typed goes into it, and is
    /// saved here first.
    let editorOpen: Bool
    /// The text on each keystroke, and nil once editing ends.
    let typing: (String?) -> Void
    let save: (String) -> Void

    @State private var text: String
    /// The text when editing began; nil when not editing. Only a change from
    /// it is saved, so clicking in and out writes nothing.
    @State private var started: String?
    @State private var hovering = false
    @FocusState private var focused: Bool

    init(notes: String, editorOpen: Bool, typing: @escaping (String?) -> Void, save: @escaping (String) -> Void) {
        self.notes = notes
        self.editorOpen = editorOpen
        self.typing = typing
        self.save = save
        _text = State(initialValue: notes)
    }

    var body: some View {
        TextField("Add a note", text: $text, axis: .vertical)
            .textFieldStyle(.plain)
            .focused($focused)
            .onSubmit { focused = false }
            .onExitCommand {
                text = started ?? notes
                focused = false
            }
            .onChange(of: focused) {
                if focused { started = text } else { finish() }
            }
            .onChange(of: text) {
                if started != nil { typing(text) }
            }
            .onChange(of: notes) {
                if started == nil { text = notes }
            }
            .onChange(of: editorOpen) {
                if editorOpen { focused = false }
            }
            .onDisappear(perform: finish)
            .padding(.horizontal, 6)
            .padding(.vertical, 4)
            .background(
                RoundedRectangle(cornerRadius: 6, style: .continuous)
                    .fill(focused ? Palette.field : hovering ? Palette.window : .clear)
                    .strokeBorder(focused ? Palette.accent : .clear))
            // The text lines up with the other rows; the field's edge sits
            // outside it.
            .padding(.horizontal, -6)
            .padding(.vertical, -4)
            .onHover { hovering = $0 }
    }

    /// Ends editing: saves the text if it was changed, else shows the notes
    /// as they are now (another app may have changed them meanwhile).
    private func finish() {
        guard let started else { return }
        self.started = nil
        typing(nil)
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if text != started && trimmed != notes {
            save(trimmed)
        } else {
            text = notes
        }
    }
}

/// Ends editing a field in the key window: a note being typed saves, and a
/// name's menu closes. The list and the toolbar's search keep the keyboard.
@MainActor
private func endEditing() {
    guard let window = NSApp.keyWindow,
          let editor = window.firstResponder as? NSTextView, editor.isFieldEditor,
          !(editor.delegate is NSSearchField)
    else { return }
    window.makeFirstResponder(nil)
}

private extension View {
    /// A click on empty space here ends editing, as it does in AppKit forms.
    func endsEditingOnClick() -> some View {
        background {
            Color.clear
                .contentShape(Rectangle())
                .onTapGesture(perform: endEditing)
        }
    }
}

// MARK: - A dictionary word

private struct CandidatePreview: View {
    let model: LibraryModel
    let candidate: CandidateView

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                Headword(
                    simplified: candidate.simplified, traditional: candidate.traditional,
                    reading: model.preferences.pinyin(raw: candidate.pinyin, display: candidate.pinyinDisplay))
                if candidate.basis == .containedWord {
                    Chip(text: "Part of “\(model.query)”, which isn’t in the dictionary", icon: "exclamationmark.triangle", style: .warn)
                } else if candidate.inferred {
                    Chip(text: "Has the same characters, but is another word", icon: "exclamationmark.triangle", style: .warn)
                }
                VStack(alignment: .leading, spacing: 8) {
                    CardLabel("Dictionary")
                    Senses(glosses: candidate.glosses)
                        .padding(.horizontal, 16)
                        .padding(.vertical, 6)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .card()
                }
                HStack(spacing: 10) {
                    Button {
                        model.add(candidate)
                    } label: {
                        Label("Add to Vocabulary", systemImage: "plus")
                    }
                    .buttonStyle(.borderedProminent)
                    .keyboardShortcut(.defaultAction)
                    Menu("Add to Collection") {
                        ForEach(model.collections) { collection in
                            Button(collection.name) { model.add(candidate, collection: collection.name) }
                        }
                        if !model.collections.isEmpty { Divider() }
                        Button("New Collection…") {
                            model.naming = .init(purpose: .newCollectionFor(candidate), text: "")
                        }
                    }
                    .fixedSize()
                    Button("Add as Needs Review") { model.add(candidate, needsReview: true) }
                }
                .controlSize(.large)
            }
            .padding(.horizontal, 32)
            .padding(.vertical, 26)
            .frame(maxWidth: 760, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

// MARK: - Several words

private struct BulkPanel: View {
    let model: LibraryModel
    let items: [ItemView]
    @State private var addingTag = false
    @State private var confirmingDelete = false

    private var ids: [Int64] { items.map(\.id) }
    private var trashed: Bool { items.allSatisfy { $0.lifecycle == .trashed } }

    /// Tags every selected word already has.
    private var sharedTags: [String] {
        guard let first = items.first else { return [] }
        return first.tags.filter { tag in items.allSatisfy { $0.tags.contains(tag) } }
    }

    /// Names any selected word has, to take off all of them.
    private func held(_ names: KeyPath<ItemView, [String]>) -> [String] {
        Set(items.flatMap { $0[keyPath: names] }).sorted { $0.localizedStandardCompare($1) == .orderedAscending }
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                Text("\(items.count) words selected").font(.system(size: 22, weight: .semibold))
                FlowLayout(spacing: 6) {
                    ForEach(items.prefix(40)) { Chip(text: $0.simplified, style: .plain) }
                    if items.count > 40 { Chip(text: "+\(items.count - 40) more", style: .plain) }
                }
                VStack(spacing: 0) {
                    if trashed {
                        action("Put Back", icon: "arrow.uturn.backward") { model.perform(.restore, on: ids) }
                        Divider()
                        action("Delete Immediately…", icon: "trash.slash", role: .destructive) { confirmingDelete = true }
                    } else {
                        action("Add Tag…", icon: "tag") { addingTag = true }
                            .popover(isPresented: $addingTag, arrowEdge: .trailing) {
                                TagPicker(tags: model.tags, shared: sharedTags, count: items.count) { tag in
                                    addingTag = false
                                    if let tag { model.perform(.addTags([tag]), on: ids) }
                                }
                            }
                        let tagged = held(\.tags)
                        if !tagged.isEmpty {
                            Divider()
                            removeMenu("Remove Tag", icon: "tag.slash", names: tagged) { .removeTags([$0]) }
                        }
                        Divider()
                        Menu {
                            ForEach(model.collections) { collection in
                                Button(collection.name) { model.perform(.addToCollection(collection.name), on: ids) }
                            }
                            if !model.collections.isEmpty { Divider() }
                            Button("New Collection…") {
                                model.naming = .init(purpose: .newCollection(then: ids), text: "")
                            }
                        } label: {
                            actionLabel("Add to Collection…", icon: "folder.badge.plus")
                        }
                        .menuStyle(.button)
                        .buttonStyle(.plain)
                        .menuIndicator(.hidden)
                        let collected = held(\.collections)
                        if !collected.isEmpty {
                            Divider()
                            removeMenu("Remove from Collection", icon: "folder.badge.minus", names: collected) {
                                .removeFromCollection($0)
                            }
                        }
                        Divider()
                        if items.allSatisfy({ $0.verification == .needsReview }) {
                            action("Mark as Confirmed", icon: "checkmark.circle") {
                                model.perform(.setVerification(.confirmed), on: ids)
                            }
                        } else {
                            action("Mark as Needs Review", icon: "exclamationmark.triangle") {
                                model.perform(.setVerification(.needsReview), on: ids)
                            }
                        }
                        Divider()
                        if items.allSatisfy({ $0.lifecycle == .archived }) {
                            action("Unarchive", icon: "tray.and.arrow.up") { model.perform(.unarchive, on: ids) }
                        } else {
                            action("Archive", icon: "archivebox") { model.perform(.archive, on: ids) }
                        }
                        Divider()
                        action("Export \(items.count) Words…", icon: "square.and.arrow.up") { model.transfer = .exporting }
                        Divider()
                        action("Move to Trash", icon: "trash", role: .destructive) { model.perform(.trash, on: ids) }
                    }
                }
                .card()
            }
            .padding(.horizontal, 32)
            .padding(.vertical, 26)
            .frame(maxWidth: 560, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .confirmationDialog(
            "Delete \(items.count) words for good?", isPresented: $confirmingDelete
        ) {
            Button("Delete Immediately", role: .destructive) { model.perform(.purge, on: ids) }
        } message: {
            Text("This can’t be undone.")
        }
    }

    /// A menu of `names`; choosing one takes it off every selected word that
    /// has it.
    private func removeMenu(
        _ title: String, icon: String, names: [String], action: @escaping (String) -> BulkAction
    ) -> some View {
        Menu {
            ForEach(names, id: \.self) { name in
                Button(name) { model.perform(action(name), on: ids) }
            }
        } label: {
            actionLabel(title, icon: icon)
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
    }

    private func action(
        _ title: String, icon: String, role: ButtonRole? = nil, perform: @escaping () -> Void
    ) -> some View {
        Button(role: role, action: perform) {
            actionLabel(title, icon: icon, destructive: role == .destructive)
        }
        .buttonStyle(.plain)
    }

    private func actionLabel(_ title: String, icon: String, destructive: Bool = false) -> some View {
        HStack(spacing: 12) {
            Image(systemName: icon)
                .frame(width: 20)
                .foregroundStyle(destructive ? Color.red : Palette.accent)
            Text(title).foregroundStyle(destructive ? Color.red : Palette.text)
            Spacer()
        }
        .font(.system(size: 13.5))
        .padding(.horizontal, 16)
        .frame(height: 40)
        .contentShape(Rectangle())
    }
}

/// Type a tag; pick an existing one or create it (mockup 03). `apply` gets
/// nil when Esc closes it.
private struct TagPicker: View {
    let tags: [GroupView]
    let shared: [String]
    let count: Int
    let apply: (String?) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Add tag to \(count) words").font(.headline)
            NameInput(
                kind: .tag, known: tags, present: shared, placement: .list, focusOnAppear: true,
                choose: apply, dismiss: { apply(nil) })
        }
        .padding(14)
        .frame(width: 280)
    }
}

/// The soft accent button from the mockups: `Replace my definition`, `Use`.
struct SoftButtonStyle: ButtonStyle {
    @Environment(\.isEnabled) private var isEnabled

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 12, weight: .semibold))
            .foregroundStyle(Palette.accent)
            .padding(.horizontal, 12)
            .frame(height: 26)
            .background(Capsule().fill(Palette.accentSoft))
            .opacity(isEnabled ? (configuration.isPressed ? 0.7 : 1) : 0.45)
    }
}
