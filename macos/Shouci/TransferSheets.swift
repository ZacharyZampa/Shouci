import ShouciCore
import SwiftUI

/// The sheet behind Import… or Export…
struct TransferSheet: View {
    let library: LibraryModel
    let transfer: LibraryModel.Transfer

    var body: some View {
        switch transfer {
        case .importing: ImportSheet(model: ImportModel(library: library))
        case .exporting: ExportSheet(model: ExportModel(library: library))
        }
    }
}

// MARK: - Import (mockup 10)

struct ImportSheet: View {
    @State var model: ImportModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            SheetTitle("Import Vocabulary", "Bring in words from Pleco or Anki.")
                .padding(.horizontal, 28)
                .padding(.top, 26)
                .padding(.bottom, 16)
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    if let summary = model.summary {
                        ImportDone(summary: summary)
                    } else {
                        fileCard
                        if model.file != nil { details }
                    }
                }
                .padding(.horizontal, 28)
                .padding(.bottom, 16)
            }
            footer
                .padding(.horizontal, 28)
                .padding(.vertical, 16)
                .overlay(alignment: .top) { Divider() }
        }
        .sheetFrame(width: 600, height: model.file == nil || model.summary != nil ? 340 : 760)
    }

    private var fileCard: some View {
        HStack(spacing: 12) {
            Image(systemName: "doc.text").font(.system(size: 24)).foregroundStyle(Palette.accent)
            VStack(alignment: .leading, spacing: 1) {
                if let file = model.file {
                    Text(file.lastPathComponent).font(Typography.emphasis).lineLimit(1)
                    Text(fileDetail).font(Typography.supporting).foregroundStyle(Palette.tertiary)
                } else {
                    Text("No file chosen").font(Typography.emphasis)
                    Text("Export from Pleco (Import/Export › Export Cards) or Anki (File › Export › Notes in Plain Text), then choose the file here.")
                        .font(Typography.supporting).foregroundStyle(Palette.tertiary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer()
            Button(model.file == nil ? "Choose File…" : "Choose…", action: model.chooseFile)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
        .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(Palette.detail))
        .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(Palette.line))
    }

    private var fileDetail: String {
        var parts: [String] = []
        if let size = model.fileSize { parts.append(size) }
        if let lines = model.plan?.counts.lines { parts.append(lines == 1 ? "1 entry found" : "\(lines) entries found") }
        return parts.joined(separator: " · ")
    }

    @ViewBuilder
    private var details: some View {
        HStack(spacing: 12) {
            Text("Format").font(Typography.content).foregroundStyle(Palette.tertiary).frame(width: 60, alignment: .leading)
            Picker("Format", selection: $model.connector) {
                ForEach(model.formats) { Text("\($0.name) text").tag(Optional($0.id)) }
            }
            .labelsHidden()
            .fixedSize()
            if model.detected { Chip(text: "Detected", style: .ok) }
            Spacer()
            if model.isWorking { ProgressView().controlSize(.small) }
        }
        if let problem = model.problem {
            Notice(text: problem)
        }
        if model.plan != nil {
            HStack(spacing: 10) {
                Tally(number: model.newWords, label: "New words")
                Tally(number: model.alreadySaved, label: "Already in your vocabulary")
                Tally(number: model.needsReview, label: "Need review", warn: model.needsReview > 0)
            }
            .fixedSize(horizontal: false, vertical: true)
            if !model.errorLines.isEmpty { errors }
            policy
            previewTable
        }
    }

    private var errors: some View {
        VStack(alignment: .leading, spacing: 6) {
            Label(
                model.errorLines.count == 1 ? "1 line can’t be imported" : "\(model.errorLines.count) lines can’t be imported",
                systemImage: "exclamationmark.triangle"
            )
            .font(Typography.emphasis)
            .foregroundStyle(Palette.warn)
            ForEach(model.errorLines.prefix(3), id: \.line) { issue in
                Text("Line \(issue.line): \(issue.message)").font(Typography.supporting).foregroundStyle(Palette.secondary)
            }
            if model.errorLines.count > 3 {
                Text("and \(model.errorLines.count - 3) more").font(Typography.supporting).foregroundStyle(Palette.tertiary)
            }
            Toggle("Skip these lines and import the rest", isOn: $model.skipErrors)
                .font(Typography.content)
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(Palette.warnSoft))
    }

    private static let policies: [(policy: ImportPolicy, title: String, detail: String)] = [
        (.skip, "Skip", "Keep my version, and add the file’s tags and collections."),
        (.merge, "Merge", "Fill in missing fields and add tags and collections. Keeps what you have edited."),
        (.overwrite, "Overwrite", "Replace my entry’s fields with the ones the file has."),
    ]

    private var policy: some View {
        VStack(alignment: .leading, spacing: 8) {
            CardLabel("If a word already exists")
            VStack(spacing: 0) {
                ForEach(Array(Self.policies.enumerated()), id: \.element.policy) { index, choice in
                    if index > 0 { Divider() }
                    PolicyRow(policy: choice.policy, selection: $model.policy, title: choice.title, detail: choice.detail)
                }
            }
            .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(Palette.line))
            // The rows are drawn to hold a sentence each; VoiceOver gets the
            // radio group they stand for.
            .accessibilityRepresentation {
                Picker("If a word already exists", selection: $model.policy) {
                    ForEach(Self.policies, id: \.policy) { Text("\($0.title). \($0.detail)").tag($0.policy) }
                }
                .pickerStyle(.radioGroup)
            }
        }
    }

    private var previewTable: some View {
        VStack(alignment: .leading, spacing: 8) {
            CardLabel("Preview")
            VStack(spacing: 0) {
                HStack {
                    Text("Word").frame(width: 120, alignment: .leading)
                    Text("Pinyin").frame(width: 120, alignment: .leading)
                    Text("Result")
                    Spacer()
                }
                .font(Typography.label)
                .foregroundStyle(Palette.tertiary)
                .padding(.horizontal, 14)
                .padding(.vertical, 7)
                .background(Palette.detail)
                Divider()
                ScrollView {
                    LazyVStack(spacing: 0) {
                        ForEach(model.plan?.lines ?? [], id: \.line) { line in
                            PlannedRow(line: line, library: model.library)
                            Divider()
                        }
                    }
                }
            }
            .frame(height: 220)
            .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(Palette.line))
        }
    }

    private var footer: some View {
        HStack(spacing: 10) {
            if model.summary == nil, model.plan != nil {
                Text(outcome)
                    .font(Typography.supporting)
                    .foregroundStyle(Palette.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer()
            if model.summary != nil {
                Button("Done") { dismiss() }.keyboardShortcut(.defaultAction)
            } else {
                Button("Cancel", role: .cancel) { dismiss() }.keyboardShortcut(.cancelAction)
                Button(model.changing == 1 ? "Import 1 Word" : "Import \(model.changing) Words", action: model.apply)
                    .keyboardShortcut(.defaultAction)
                    .disabled(!model.canImport)
            }
        }
    }

    private var outcome: String {
        if model.plan?.refused == true { return "Nothing is imported while the file has lines with errors." }
        var parts: [String] = []
        if model.newWords > 0 { parts.append("\(model.newWords) will be added") }
        if model.updates > 0 {
            let updated = switch model.policy {
            case .overwrite: "\(model.updates) updated"
            case .merge: "\(model.updates) merged"
            case .skip: "\(model.updates) get the file’s tags and collections"
            }
            parts.append(updated)
        }
        if model.needsReview > 0 { parts.append("\(model.needsReview) added as needs review") }
        if parts.isEmpty { return "Nothing in this file changes your vocabulary." }
        return Self.sentence(parts) + "."
    }

    static func sentence(_ parts: [String]) -> String {
        switch parts.count {
        case 0: return ""
        case 1: return parts[0].prefix(1).uppercased() + parts[0].dropFirst()
        case 2:
            let text = parts[0] + " and " + parts[1]
            return text.prefix(1).uppercased() + text.dropFirst()
        default:
            let text = parts.dropLast().joined(separator: ", ") + ", and " + parts[parts.count - 1]
            return text.prefix(1).uppercased() + text.dropFirst()
        }
    }
}

private struct PolicyRow: View {
    let policy: ImportPolicy
    @Binding var selection: ImportPolicy
    let title: String
    let detail: String

    var body: some View {
        Button {
            selection = policy
        } label: {
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                Image(systemName: selection == policy ? "largecircle.fill.circle" : "circle")
                    .foregroundStyle(selection == policy ? Palette.accentFill : Palette.tertiary)
                VStack(alignment: .leading, spacing: 1) {
                    Text(title).font(Typography.emphasis)
                    Text(detail).font(Typography.supporting).foregroundStyle(Palette.secondary)
                }
                Spacer()
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
            .background(selection == policy ? Palette.accentSoft : Color.clear)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

/// One line of the file and what importing does with it.
private struct PlannedRow: View {
    let line: PlannedLine
    let library: LibraryModel

    var body: some View {
        let (word, pinyin, result, tone) = describe()
        HStack {
            Text(word).font(Typography.emphasis).chinese().lineLimit(1).frame(width: 120, alignment: .leading)
            Text(pinyin).foregroundStyle(Palette.secondary).lineLimit(1).frame(width: 120, alignment: .leading)
            Text(result)
                .foregroundStyle(tone)
                .fontWeight(tone == Palette.secondary ? .regular : .semibold)
                .lineLimit(1)
            Spacer()
        }
        .font(Typography.content)
        .padding(.horizontal, 14)
        .frame(height: 32)
    }

    private func describe() -> (String, String, String, Color) {
        let preferences = library.preferences
        func reading(_ raw: String) -> String {
            raw.isEmpty ? "—" : preferences.pinyin(raw: raw, display: toneMarks(pinyin: raw))
        }
        switch line.action {
        case .insert(let word):
            return word.verification == .needsReview
                ? (word.simplified, reading(word.pinyin), "New · needs review", Palette.warn)
                : (word.simplified, reading(word.pinyin), "New", Palette.secondary)
        case .update(let id, let simplified, _, let changes, let conflicts, let tags, let collections, let restore):
            let saved = library.item(id).map { reading($0.pinyin) } ?? "—"
            var what: [String] = []
            if restore { what.append("back from the Trash") }
            if !changes.isEmpty { what.append("updates \(changes.map(\.field.name).joined(separator: ", "))") }
            if !tags.add.isEmpty || !collections.add.isEmpty { what.append("adds \((tags.add + collections.add).joined(separator: ", "))") }
            if !conflicts.isEmpty { what.append("keeps your \(conflicts.map(\.field.name).joined(separator: ", "))") }
            return (simplified, saved, "Already saved · " + what.joined(separator: "; "), Palette.accent)
        case .skip(let id, let simplified, _, let reason, _):
            let saved = id.flatMap(library.item).map { reading($0.pinyin) } ?? "—"
            let text = switch reason {
            case .alreadySaved: "Already saved · skipped"
            case .inTrash: "In the Trash · skipped"
            case .unchanged: "Already saved · nothing new"
            case .repeatedInFile: "Repeats an earlier line"
            }
            return (simplified, saved, text, Palette.secondary)
        case .drop(let reason):
            let word = line.raw.split(separator: "\t").first.map(String.init) ?? ""
            return (word.isEmpty ? "—" : word, "—", "Line \(line.line) · \(reason)", Palette.warn)
        }
    }
}

private struct ImportDone: View {
    let summary: TransferSummary

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Label(title, systemImage: "checkmark.circle")
                .font(Typography.emphasis)
                .foregroundStyle(Palette.ok)
            Text(detail).font(Typography.content).foregroundStyle(Palette.secondary)
            ForEach(summary.notes, id: \.self) { note in
                Text(note).font(Typography.supporting).foregroundStyle(Palette.tertiary)
            }
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(Palette.okSoft))
    }

    private var title: String {
        let count = summary.inserted + summary.updated
        return count == 1 ? "Imported 1 word" : "Imported \(count) words"
    }

    private var detail: String {
        var parts: [String] = []
        if summary.inserted > 0 { parts.append("\(summary.inserted) added") }
        if summary.updated > 0 { parts.append("\(summary.updated) updated") }
        if summary.unresolved > 0 { parts.append("\(summary.unresolved) need review") }
        if summary.skipped > 0 { parts.append("\(summary.skipped) skipped") }
        if summary.dropped > 0 { parts.append("\(summary.dropped) lines left out") }
        return ImportSheet.sentence(parts) + "."
    }
}

// MARK: - Export (mockup 11)

struct ExportSheet: View {
    @State var model: ExportModel
    @State private var confirmingReplace = false
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            SheetTitle("Export Vocabulary", "Send words to Pleco or Anki.")
                .padding(.horizontal, 28)
                .padding(.top, 26)
                .padding(.bottom, 18)
            // Scrolls only when the screen is shorter than the sheet.
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    if let summary = model.summary {
                        done(summary)
                    } else {
                        destinationCards
                        choices
                        include
                        saveAs
                        if let problem = model.problem {
                            Notice(text: problem)
                        }
                    }
                }
                .padding(.horizontal, 28)
                .padding(.bottom, 18)
            }
            .scrollBounceBehavior(.basedOnSize)
            footer
                .padding(.horizontal, 28)
                .padding(.bottom, 22)
        }
        .sheetFrame(width: 600, height: model.summary == nil ? 640 : 300)
        .task { model.start() }
    }

    private var destinationCards: some View {
        VStack(alignment: .leading, spacing: 8) {
            CardLabel("Destination")
            HStack(spacing: 10) {
                ForEach(model.formats) { format in
                    Button {
                        model.connector = format.id
                    } label: {
                        HStack(alignment: .firstTextBaseline, spacing: 10) {
                            Image(systemName: model.connector == format.id ? "largecircle.fill.circle" : "circle")
                                .foregroundStyle(model.connector == format.id ? Palette.accentFill : Palette.tertiary)
                            VStack(alignment: .leading, spacing: 1) {
                                Text(format.name).font(Typography.emphasis)
                                Text(Self.blurb(format.id)).font(Typography.supporting).foregroundStyle(Palette.secondary)
                            }
                            Spacer()
                        }
                        .padding(.horizontal, 14)
                        .padding(.vertical, 12)
                        .background(
                            RoundedRectangle(cornerRadius: 12, style: .continuous)
                                .fill(model.connector == format.id ? Palette.accentSoft : Color.clear))
                        .overlay(
                            RoundedRectangle(cornerRadius: 12, style: .continuous)
                                .strokeBorder(model.connector == format.id ? Palette.accentFill : Palette.line,
                                              lineWidth: model.connector == format.id ? 2 : 1))
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
            }
            .accessibilityRepresentation {
                Picker("Destination", selection: $model.connector) {
                    ForEach(model.formats) { Text("\($0.name). \(Self.blurb($0.id))").tag($0.id) }
                }
                .pickerStyle(.radioGroup)
            }
        }
    }

    private static func blurb(_ connector: String) -> String {
        switch connector {
        case "anki": "Text file to import into a deck"
        case "pleco": "UTF-8 flashcard text"
        default: "Plain text"
        }
    }

    private var choices: some View {
        VStack(alignment: .leading, spacing: 8) {
            CardLabel("What to export")
            VStack(spacing: 0) {
                ForEach(Array(model.choices.enumerated()), id: \.element) { index, choice in
                    if index > 0 { Divider() }
                    choiceRow(choice)
                }
            }
            .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(Palette.line))
            .accessibilityRepresentation {
                Picker("What to export", selection: $model.choice) {
                    ForEach(model.choices, id: \.self) { Text(spoken($0)).tag($0) }
                }
                .pickerStyle(.radioGroup)
            }
        }
    }

    /// A choice as VoiceOver reads it: `Current view, HSK 1. 12 words`.
    private func spoken(_ choice: ExportChoice) -> String {
        var title = title(choice)
        if choice == .filter, let label = model.filter?.label { title += ", \(label)" }
        return [title, detail(choice), wordCount(choice)].compactMap { $0 }.joined(separator: ". ")
    }

    private func wordCount(_ choice: ExportChoice) -> String? {
        model.counts[choice].map { $0 == 1 ? "1 word" : "\($0) words" }
    }

    private func choiceRow(_ choice: ExportChoice) -> some View {
        let selected = model.choice == choice
        return Button {
            model.choice = choice
        } label: {
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                Image(systemName: selected ? "largecircle.fill.circle" : "circle")
                    .foregroundStyle(selected ? Palette.accentFill : Palette.tertiary)
                VStack(alignment: .leading, spacing: 1) {
                    HStack(spacing: 8) {
                        Text(title(choice)).font(Typography.emphasis)
                        if choice == .filter, let label = model.filter?.label { Chip(text: label) }
                    }
                    if let detail = detail(choice) {
                        Text(detail).font(Typography.supporting).foregroundStyle(Palette.secondary)
                    }
                }
                Spacer()
                Text(wordCount(choice) ?? "–")
                    .font(Typography.content)
                    .foregroundStyle(Palette.secondary)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 11)
            .background(selected ? Palette.accentSoft : Color.clear)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    private func title(_ choice: ExportChoice) -> String {
        switch choice {
        case .new: "New since last export"
        case .all: "Everything"
        case .selected: "Selected words"
        case .filter: "Current view"
        }
    }

    private func detail(_ choice: ExportChoice) -> String? {
        switch choice {
        case .new: "Words \(model.formatName) doesn’t have yet: never exported there or imported from it"
        default: nil
        }
    }

    private var include: some View {
        VStack(alignment: .leading, spacing: 8) {
            CardLabel("Include")
            Toggle("Words marked needs review", isOn: $model.includeNeedsReview).font(Typography.content)
            if model.connector == "anki" {
                HStack(spacing: 12) {
                    Text("Deck").font(Typography.content).foregroundStyle(Palette.tertiary)
                    TextField("Anki asks when you import", text: $model.deck)
                        .textFieldStyle(.roundedBorder)
                        .accessibilityLabel("Deck")
                }
            }
        }
    }

    private var saveAs: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 12) {
                Text("Save as").font(Typography.content).foregroundStyle(Palette.tertiary)
                HStack(spacing: 8) {
                    Image(systemName: "doc").foregroundStyle(Palette.tertiary)
                    Text(model.destination.lastPathComponent).lineLimit(1)
                    Spacer()
                    Text(model.destination.deletingLastPathComponent().lastPathComponent)
                        .foregroundStyle(Palette.tertiary)
                        .lineLimit(1)
                }
                .font(Typography.content)
                .padding(.horizontal, 12)
                .frame(height: 30)
                .background(RoundedRectangle(cornerRadius: 8, style: .continuous).fill(Palette.field))
                .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).strokeBorder(Palette.line))
                Button("Choose…", action: model.chooseDestination)
            }
            if model.plan?.replacesExisting == true {
                Text("A file with this name is already there and will be replaced.")
                    .font(Typography.control)
                    .foregroundStyle(Palette.warn)
            }
        }
    }

    private var footer: some View {
        HStack(spacing: 10) {
            if model.summary == nil {
                Text(outcome)
                    .font(Typography.supporting)
                    .foregroundStyle(Palette.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer()
            if model.summary != nil {
                Button("Show in Finder", action: model.showInFinder)
                Button("Done") { dismiss() }.keyboardShortcut(.defaultAction)
            } else {
                Button("Cancel", role: .cancel) { dismiss() }.keyboardShortcut(.cancelAction)
                Button(model.count == 1 ? "Export 1 Word" : "Export \(model.count) Words") {
                    if model.needsReplaceConfirmation {
                        confirmingReplace = true
                    } else {
                        model.apply()
                    }
                }
                .keyboardShortcut(.defaultAction)
                .disabled(!model.canExport)
                .confirmationDialog(
                    "Replace “\(model.destination.lastPathComponent)”?",
                    isPresented: $confirmingReplace
                ) {
                    Button("Replace", role: .destructive, action: model.apply)
                    Button("Cancel", role: .cancel) {}
                } message: {
                    Text("A file with this name is already in \(model.destination.deletingLastPathComponent().lastPathComponent). Exporting replaces it.")
                }
            }
        }
    }

    private var outcome: String {
        guard let plan = model.plan else { return model.isWorking ? "Counting words…" : "" }
        var parts: [String] = [
            plan.itemIds.count == 1
                ? "1 word will be written for \(model.formatName)"
                : "\(plan.itemIds.count) words will be written for \(model.formatName)"
        ]
        if plan.leftOutNeedsReview > 0 { parts.append("\(plan.leftOutNeedsReview) left out because they need review") }
        if plan.leftOutAlreadyThere > 0 { parts.append("\(plan.leftOutAlreadyThere) already in \(model.formatName)") }
        var text = parts.joined(separator: "; ") + "."
        if !plan.notes.isEmpty { text += " " + plan.notes.joined(separator: " ") }
        return text
    }

    private func done(_ summary: TransferSummary) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            Label(
                summary.written == 1 ? "Exported 1 word" : "Exported \(summary.written) words",
                systemImage: "checkmark.circle"
            )
            .font(Typography.emphasis)
            .foregroundStyle(Palette.ok)
            Text("Saved to \(model.destination.lastPathComponent) in \(model.destination.deletingLastPathComponent().lastPathComponent).")
                .font(Typography.content)
                .foregroundStyle(Palette.secondary)
            Text(model.connector == "anki"
                 ? "In Anki, choose File › Import… and pick this file."
                 : "In Pleco, open Import/Export › Import Cards and pick this file.")
                .font(Typography.content)
                .foregroundStyle(Palette.secondary)
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(Palette.okSoft))
    }
}

// MARK: - Pieces

private struct SheetTitle: View {
    let title: String
    let subtitle: String

    init(_ title: String, _ subtitle: String) {
        self.title = title
        self.subtitle = subtitle
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(title).font(Typography.title)
            Text(subtitle).font(Typography.content).foregroundStyle(Palette.secondary)
        }
    }
}

private struct Tally: View {
    let number: Int
    let label: String
    var warn = false

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text("\(number)").font(Typography.figure).foregroundStyle(warn ? Palette.warn : Palette.text)
            Text(label).font(Typography.supporting).foregroundStyle(warn ? Palette.text : Palette.secondary)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(warn ? Palette.warnSoft : Palette.detail))
        .overlay {
            if !warn { RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(Palette.line) }
        }
        .accessibilityElement(children: .combine)
    }
}
