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
        .frame(width: 600, height: model.file == nil || model.summary != nil ? 340 : 760)
    }

    private var fileCard: some View {
        HStack(spacing: 12) {
            Image(systemName: "doc.text").font(.system(size: 24)).foregroundStyle(Palette.accent)
            VStack(alignment: .leading, spacing: 1) {
                if let file = model.file {
                    Text(file.lastPathComponent).font(.system(size: 14, weight: .semibold)).lineLimit(1)
                    Text(fileDetail).font(.system(size: 12.5)).foregroundStyle(Palette.tertiary)
                } else {
                    Text("No file chosen").font(.system(size: 14, weight: .semibold))
                    Text("Export from Pleco (Import/Export › Export Cards) or Anki (File › Export › Notes in Plain Text), then choose the file here.")
                        .font(.system(size: 12.5)).foregroundStyle(Palette.tertiary)
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
            Text("Format").font(.system(size: 13)).foregroundStyle(Palette.tertiary).frame(width: 60, alignment: .leading)
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
            Notice(icon: "exclamationmark.triangle", text: problem)
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
            .font(.system(size: 13, weight: .semibold))
            .foregroundStyle(Palette.warn)
            ForEach(model.errorLines.prefix(3), id: \.line) { issue in
                Text("Line \(issue.line): \(issue.message)").font(.system(size: 12.5)).foregroundStyle(Palette.secondary)
            }
            if model.errorLines.count > 3 {
                Text("and \(model.errorLines.count - 3) more").font(.system(size: 12.5)).foregroundStyle(Palette.tertiary)
            }
            Toggle("Skip these lines and import the rest", isOn: $model.skipErrors)
                .font(.system(size: 13))
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(Palette.warnSoft))
    }

    private var policy: some View {
        VStack(alignment: .leading, spacing: 8) {
            CardLabel("If a word already exists")
            VStack(spacing: 0) {
                PolicyRow(
                    policy: .skip, selection: $model.policy, title: "Skip",
                    detail: "Keep my version. Imported duplicates are ignored.")
                Divider()
                PolicyRow(
                    policy: .merge, selection: $model.policy, title: "Merge",
                    detail: "Fill in missing fields and add tags and collections. Keeps what you have edited.")
                Divider()
                PolicyRow(
                    policy: .overwrite, selection: $model.policy, title: "Overwrite",
                    detail: "Replace my entry’s fields with the ones the file has.")
            }
            .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(Palette.line))
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
                .font(.system(size: 11.5))
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
                    .font(.system(size: 12.5))
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
            parts.append(model.policy == .overwrite ? "\(model.updates) updated" : "\(model.updates) merged")
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
                    Text(title).font(.system(size: 13.5, weight: .semibold))
                    Text(detail).font(.system(size: 12.5)).foregroundStyle(Palette.secondary)
                }
                Spacer()
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
            .background(selection == policy ? Palette.accentSoft : Color.clear)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selection == policy ? .isSelected : [])
    }
}

/// One line of the file and what importing does with it.
private struct PlannedRow: View {
    let line: PlannedLine
    let library: LibraryModel

    var body: some View {
        let (word, pinyin, result, tone) = describe()
        HStack {
            Text(word).font(.system(size: 15, weight: .semibold)).lineLimit(1).frame(width: 120, alignment: .leading)
            Text(pinyin).foregroundStyle(Palette.secondary).lineLimit(1).frame(width: 120, alignment: .leading)
            Text(result)
                .foregroundStyle(tone)
                .fontWeight(tone == Palette.secondary ? .regular : .semibold)
                .lineLimit(1)
            Spacer()
        }
        .font(.system(size: 13))
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
            if !changes.isEmpty { what.append("updates \(changes.map(\.field).joined(separator: ", "))") }
            if !tags.add.isEmpty || !collections.add.isEmpty { what.append("adds \((tags.add + collections.add).joined(separator: ", "))") }
            if !conflicts.isEmpty { what.append("keeps your \(conflicts.map(\.field).joined(separator: ", "))") }
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
                .font(.system(size: 16, weight: .semibold))
                .foregroundStyle(Palette.ok)
            Text(detail).font(.system(size: 13)).foregroundStyle(Palette.secondary)
            ForEach(summary.notes, id: \.self) { note in
                Text(note).font(.system(size: 12.5)).foregroundStyle(Palette.tertiary)
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
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            SheetTitle("Export Vocabulary", "Send words to Pleco or Anki.")
            if let summary = model.summary {
                done(summary)
            } else {
                destinationCards
                choices
                include
                saveAs
                if let problem = model.problem {
                    Notice(icon: "exclamationmark.triangle", text: problem)
                }
            }
            Spacer(minLength: 0)
            footer
        }
        .padding(.horizontal, 28)
        .padding(.top, 26)
        .padding(.bottom, 22)
        .frame(width: 600)
        .frame(minHeight: model.summary == nil ? 640 : 300)
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
                                Text(format.name).font(.system(size: 14, weight: .semibold))
                                Text(Self.blurb(format.id)).font(.system(size: 12.5)).foregroundStyle(Palette.secondary)
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
                    .accessibilityAddTraits(model.connector == format.id ? .isSelected : [])
                }
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
        }
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
                        Text(title(choice)).font(.system(size: 13.5, weight: .semibold))
                        if choice == .filter, let label = model.filter?.label { Chip(text: label) }
                    }
                    if let detail = detail(choice) {
                        Text(detail).font(.system(size: 12.5)).foregroundStyle(Palette.secondary)
                    }
                }
                Spacer()
                Text(model.counts[choice].map { $0 == 1 ? "1 word" : "\($0) words" } ?? "–")
                    .font(.system(size: 13))
                    .foregroundStyle(Palette.secondary)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 11)
            .background(selected ? Palette.accentSoft : Color.clear)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? .isSelected : [])
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
            Toggle("Words marked needs review", isOn: $model.includeNeedsReview).font(.system(size: 13.5))
            if model.connector == "anki" {
                HStack(spacing: 12) {
                    Text("Deck").font(.system(size: 13)).foregroundStyle(Palette.tertiary)
                    TextField("Anki asks when you import", text: $model.deck)
                        .textFieldStyle(.roundedBorder)
                }
            }
        }
    }

    private var saveAs: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 12) {
                Text("Save as").font(.system(size: 13)).foregroundStyle(Palette.tertiary)
                HStack(spacing: 8) {
                    Image(systemName: "doc").foregroundStyle(Palette.tertiary)
                    Text(model.destination.lastPathComponent).lineLimit(1)
                    Spacer()
                    Text(model.destination.deletingLastPathComponent().lastPathComponent)
                        .foregroundStyle(Palette.tertiary)
                        .lineLimit(1)
                }
                .font(.system(size: 13))
                .padding(.horizontal, 12)
                .frame(height: 30)
                .background(RoundedRectangle(cornerRadius: 8, style: .continuous).fill(Palette.field))
                .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).strokeBorder(Palette.line))
                Button("Choose…", action: model.chooseDestination)
            }
            if model.plan?.replacesExisting == true {
                Text("A file with this name is already there and will be replaced.")
                    .font(.system(size: 12))
                    .foregroundStyle(Palette.warn)
            }
        }
    }

    private var footer: some View {
        HStack(spacing: 10) {
            if model.summary == nil {
                Text(outcome)
                    .font(.system(size: 12.5))
                    .foregroundStyle(Palette.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer()
            if model.summary != nil {
                Button("Show in Finder", action: model.showInFinder)
                Button("Done") { dismiss() }.keyboardShortcut(.defaultAction)
            } else {
                Button("Cancel", role: .cancel) { dismiss() }.keyboardShortcut(.cancelAction)
                Button(model.count == 1 ? "Export 1 Word" : "Export \(model.count) Words", action: model.apply)
                    .keyboardShortcut(.defaultAction)
                    .disabled(!model.canExport)
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
            .font(.system(size: 16, weight: .semibold))
            .foregroundStyle(Palette.ok)
            Text("Saved to \(model.destination.lastPathComponent) in \(model.destination.deletingLastPathComponent().lastPathComponent).")
                .font(.system(size: 13))
                .foregroundStyle(Palette.secondary)
            Text(model.connector == "anki"
                 ? "In Anki, choose File › Import… and pick this file."
                 : "In Pleco, open Import/Export › Import Cards and pick this file.")
                .font(.system(size: 13))
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
            Text(title).font(.system(size: 20, weight: .semibold))
            Text(subtitle).font(.system(size: 13)).foregroundStyle(Palette.secondary)
        }
    }
}

private struct Tally: View {
    let number: Int
    let label: String
    var warn = false

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text("\(number)").font(.system(size: 26, weight: .semibold)).foregroundStyle(warn ? Palette.warn : Palette.text)
            Text(label).font(.system(size: 12.5)).foregroundStyle(warn ? Palette.text : Palette.secondary)
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

private struct Notice: View {
    let icon: String
    let text: String

    var body: some View {
        Label(text, systemImage: icon)
            .font(.system(size: 13))
            .foregroundStyle(Palette.warn)
            .padding(12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(Palette.warnSoft))
    }
}
